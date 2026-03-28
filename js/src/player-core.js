import wasmInit, { FrameBuffer, PlaybackClock, Renderer, SubtitleEngine, PlayerState, MediaSource, StreamLoader, probe } from './pkg/videoplayer.js';
export { wasmInit };
import { DemuxerInterface } from './demuxer-interface.js';

export class HEVCPlayerCore {
    constructor(canvas, dom) {
        this.canvas = canvas;
        this.dom = dom; // { status, subtitleEl, seekbar, timeDisplay, fpsDisplay, audioTrackSelect, subTrackSelect }
        this.renderer = null;
        this.demuxer = null;
        this.worker = null;
        this.frameBuffer = null;
        this.clock = null;
        this.state = null; // PlayerState (Rust)
        this.rafId = null;
        // Audio (browser APIs — must stay JS)
        this.audioCtx = null;
        this.audioDecoder = null;
        this.audioBufferQueue = [];
        this.audioAnchorTime = 0;
        this.audioAnchorElapsed = 0;
        // Streaming
        this.streamLoader = null;
        this.isStreaming = false;
        this._fetchingData = false;
        this._lastFetchedSample = 0;
        // Subtitles (Rust engine)
        this.subtitleEngine = null;
        this._lastSubCount = 0;
        this._lastSubHtml = '';
    }

    _setStatus(text) {
        if (this.dom.status) this.dom.status.textContent = text;
    }

    _getStatus() {
        return this.dom.status?.textContent || '';
    }

    async loadStream(url) {
        this._setStatus('Initializing...');
        await wasmInit();

        this.renderer = new Renderer(this.canvas);
        this.frameBuffer = new FrameBuffer(50, 3);
        this.clock = new PlaybackClock();
        this.state = new PlayerState();
        this.updateTime(0);

        this._setStatus('Connecting...');
        // Rust StreamLoader: HEAD + probe + moov detection all in Rust
        this.streamLoader = await new StreamLoader(url);

        // Rust auto-detects format + creates demuxer
        const mediaSource = new MediaSource();
        mediaSource.init_from_bytes(this.streamLoader.init_data());
        this.demuxer = new DemuxerInterface(mediaSource);

        this.demuxer.stillDownloading = true;
        this.state.set_still_downloading(true);

        // Buffer initial data until ready
        while (true) {
            await this._bufferMore();
            const frames = this.demuxer.sampleCount();
            this._setStatus(`Buffering... ${frames} frames`);
            if (this.demuxer.headerReady() && frames >= 30) break;
            if (!this.state.still_downloading()) break;
        }

        if (!this.demuxer.headerReady()) throw new Error('Could not parse header');

        // Apply demuxer info
        this.canvas.width = this.demuxer.width();
        this.canvas.height = this.demuxer.height();
        this.state.set_total_video_samples(this.demuxer.sampleCount());
        this.state.set_duration_ms(this.demuxer.durationMs());
        this.state.set_nal_length_size(this.demuxer.nalLengthSize());
        this._setStatus(`Video: ${this.demuxer.width()}x${this.demuxer.height()}, ${(this.state.duration_ms() / 1000).toFixed(1)}s`);

        await this._initDecoder();
        await this._postInit();
    }

    async _initDecoder() {
        this._setStatus(this._getStatus() + ' — Loading decoder...');
        const workerUrl = new URL('./decode-worker.js', import.meta.url).href;
        this.worker = new Worker(workerUrl, { type: 'module' });

        await new Promise((resolve, reject) => {
            const timeout = setTimeout(() => reject(new Error('Decoder init timed out')), 10000);
            this.worker.onmessage = (e) => {
                if (e.data.type === 'ready') { clearTimeout(timeout); resolve(); }
                if (e.data.type === 'error') console.error('[decoder]', e.data.msg);
                if (e.data.type === 'log') console.log('[decoder]', e.data.msg);
            };
            this.worker.onerror = (e) => { clearTimeout(timeout); reject(e); };
            const config = this.demuxer.codecDescription();
            this.worker.postMessage({ type: 'init', codec: 'hevc', config });
        });

        this.worker.onmessage = (e) => this.onWorkerMessage(e.data);
    }

    async _postInit() {
        this._setStatus(this._getStatus().replace(/ — .*/, '') + ' — Decoding thumbnail...');
        await this.decodeFirstFrame();

        this.state.set_next_video_sample(0);
        this.state.clear_pending();
        this.frameBuffer.reset();
        const config = this.demuxer.codecDescription();
        this.worker.postMessage({ type: 'reset', config });

        if (this.demuxer.hasAudio() && typeof AudioDecoder !== 'undefined') {
            await this.initAudio();
        }

        if (this.demuxer.hasSubtitles()) {
            this.loadSubtitles();
        }

        this.populateTrackSelectors();

        this._setStatus(this._getStatus().replace(/ — .*/, '') + ' — Ready');
    }

    async decodeFirstFrame() {
        return new Promise((resolve) => {
            const prevHandler = this.worker.onmessage;
            let sent = 0;
            const maxToSend = Math.min(30, this.state.total_video_samples());

            const trySend = () => {
                while (sent < maxToSend && this.state.next_video_sample() < this.state.total_video_samples()) {
                    const sample = this.demuxer.readSample(this.state.next_video_sample());
                    this.state.advance_video_sample();
                    if (!sample) continue;
                    this.worker.postMessage({
                        type: 'samples',
                        samples: [{ data: sample.data, pts: 0 }],
                        nalLengthSize: this.state.nal_length_size(),
                    });
                    sent++;
                    break;
                }
            };

            const timeout = setTimeout(() => {
                console.warn('[thumbnail] timed out after sending', sent, 'samples');
                this.worker.onmessage = prevHandler;
                resolve();
            }, 10000);

            this.worker.onmessage = (e) => {
                const msg = e.data;
                if (msg.type === 'frame') {
                    this.frameBuffer.push(msg.pts, msg.y, msg.u, msg.v, msg.w, msg.h);
                    this.frameBuffer.pop_frame(Infinity, true);
                    this.renderer.render_current_frame(this.frameBuffer);
                    clearTimeout(timeout);
                    this.worker.onmessage = prevHandler;
                    resolve();
                } else if (msg.type === 'decoded') {
                    this.state.sub_pending(1);
                    if (msg.frames === 0) trySend();
                } else if (msg.type === 'log') {
                    console.log('[decoder]', msg.msg);
                } else if (msg.type === 'error') {
                    console.error('[decoder]', msg.msg);
                }
            };

            this.state.clear_pending();
            trySend();
        });
    }

    onWorkerMessage(msg) {
        switch (msg.type) {
            case 'frame': {
                this.frameBuffer.push(msg.pts, msg.y, msg.u, msg.v, msg.w, msg.h);
                break;
            }
            case 'decoded':
                this.state.sub_pending(msg.count);
                if (msg.avgMs && window.verbose) console.log(`[perf] ${msg.count} samples, ${msg.frames} frames, avg ${msg.avgMs}ms/sample`);
                if (this.clock.is_playing() || this._seekDecoding) this.feedWorker();
                break;
            case 'flushed':
                this.state.set_flushed(true);
                break;
            case 'ready':
                if (this._seekTarget != null) this._onSeekReady();
                break;
            case 'log':
                console.log('[decoder]', msg.msg);
                break;
            case 'error':
                console.error('[decoder]', msg.msg);
                this._setStatus(`Decoder: ${msg.msg}`);
                break;
        }
    }

    feedWorker() {
        // Update sample counts (may grow during streaming MKV)
        this.state.set_total_video_samples(this.demuxer.sampleCount());

        if (!this.state.should_feed(this.frameBuffer.len())) {
            if (this.state.should_flush()) {
                this.worker.postMessage({ type: 'flush' });
            }
            return;
        }

        const batchSize = Math.min(10, this.state.total_video_samples() - this.state.next_video_sample());
        const samples = [];

        for (let i = 0; i < batchSize; i++) {
            const sample = this.demuxer.readSample(this.state.next_video_sample());
            if (!sample) break;
            this.state.advance_video_sample();
            samples.push({
                data: sample.data,
                pts: Math.round(sample.timestamp_us),
            });
        }

        if (samples.length > 0) {
            this.state.add_pending(samples.length);
            this.worker.postMessage(
                { type: 'samples', samples, nalLengthSize: this.state.nal_length_size() }
            );
        }
    }

    play() {
        if (this.clock.is_playing()) return;
        this.clock.play(performance.now());
        if (this.audioCtx) {
            this.audioCtx.resume();
            this.audioAnchorTime = this.audioCtx.currentTime;
            this.audioAnchorElapsed = this.clock.elapsed_us(performance.now());
        }
        this.feedWorker();
        this.renderLoop();
    }

    pause() {
        this.clock.pause(performance.now());
        if (this.audioCtx) this.audioCtx.suspend();
        if (this.rafId) {
            cancelAnimationFrame(this.rafId);
            this.rafId = null;
        }
    }

    restart() {
        this.pause();
        this.frameBuffer.reset();
        this.clock.reset();
        this.state.set_next_video_sample(0);
        this.state.clear_pending();
        this.state.set_flushed(false);
        this.state.set_next_audio_sample(0);
        this.audioBufferQueue = [];
        this.audioAnchorTime = 0;
        this.audioAnchorElapsed = 0;
        if (this.audioDecoder && this.audioDecoder.state !== 'closed') {
            this.audioDecoder.reset();
            this.audioDecoder.configure({
                codec: 'mp4a.40.2',
                sampleRate: this.demuxer.audioSampleRate(),
                numberOfChannels: this.demuxer.audioChannelCount(),
                description: this.demuxer.audioCodecConfig(),
            });
        }

        const hvcc = this.demuxer.codecDescription();
        this.worker.postMessage({ type: 'reset', config: hvcc });
        this.renderer?.clear();
        this.updateTime(0);
    }

    renderLoop() {
        if (!this.clock.is_playing()) return;

        this.feedWorker();
        this.feedAudioDecoder();

        const now = performance.now();
        const elapsedUs = this.clock.elapsed_us(now);

        this.scheduleAudio(elapsedUs);

        // Reload subtitles if more arrived (streaming)
        const subCount = this.demuxer.subtitleCount();
        if (subCount > this._lastSubCount) {
            const firstSubs = this._lastSubCount === 0;
            this._lastSubCount = subCount;
            this.loadSubtitles();
            if (firstSubs) this.populateTrackSelectors(); // show dropdown
        }
        this.updateSubtitles(elapsedUs);

        // Buffer ahead — for MP4 streaming, check if upcoming samples are fetched
        if (!this._fetchingData && this.state.still_downloading()) {
            const next = this.state.next_video_sample();
            const lookAhead = Math.min(next + 240, this.state.total_video_samples());
            const needsFetch = this.isStreaming
                ? !this.demuxer.hasVideoSample(lookAhead - 1)  // MP4: sample-level check
                : this.state.needs_buffer();                     // MKV: frame count check
            if (needsFetch) this._bufferMore();
        }

        const MIN_REORDER = 3;
        let frameToShow = null;
        let skipped = 0;
        while (this.decodedFramesCount() > MIN_REORDER || this.state.flushed()) {
            if (!this.frameBuffer.pop_frame(elapsedUs, this.state.flushed())) break;
            frameToShow = true;
        }

        if (frameToShow) {
            this.renderer.render_current_frame(this.frameBuffer);
            this._fpsFrames = (this._fpsFrames || 0) + 1;
            if (!this._fpsTime) this._fpsTime = now;
            if (now - this._fpsTime >= 1000) {
                if (this.dom.fpsDisplay) this.dom.fpsDisplay.textContent = `${this._fpsFrames} fps`;
                this._fpsFrames = 0;
                this._fpsTime = now;
            }
        }

        this.updateTime(elapsedUs / 1000);

        const done = this.state.flushed() && this.frameBuffer.len() === 0;
        if (done && !this.demuxer.stillDownloading) {
            this.clock.pause(now);
            this._setStatus(this._getStatus().replace(/ — .*/, '') + ' — Finished');
        } else {
            this.rafId = requestAnimationFrame(() => this.renderLoop());
        }
    }

    decodedFramesCount() {
        return this.frameBuffer.len();
    }

    updateTime(elapsedMs) {
        const cur = Math.min(elapsedMs / 1000, this.state.duration_ms() / 1000);
        const tot = this.state.duration_ms() / 1000;
        if (this.dom.timeDisplay) this.dom.timeDisplay.textContent = `${fmtTime(cur)} / ${fmtTime(tot)}`;
        if (this.dom.seekbar && !this._seekDragging) {
            this.dom.seekbar.value = tot > 0 ? (cur / tot * 1000) : 0;
        }
    }

    setSpeed(speed) {
        if (this.audioCtx && this.audioCtx.state === 'running') {
            this.audioAnchorTime = this.audioCtx.currentTime;
            this.audioAnchorElapsed = this.clock.elapsed_us(performance.now());
        }
        this.clock.set_speed(performance.now(), speed);
    }

    // ── Seek ──

    seek(targetUs) {
        const wasPlaying = this.clock.is_playing();
        this._seekDecoding = false;
        this.pause();

        this.frameBuffer.reset();
        this.state.set_flushed(false);
        this.state.clear_pending();
        this.state.set_next_video_sample(this.demuxer.findKeyframeBefore(targetUs));
        this._lastFetchedSample = this.state.next_video_sample();

        if (this.audioDecoder && this.audioDecoder.state !== 'closed') {
            this.audioBufferQueue = [];
            this.state.set_next_audio_sample(this.demuxer.findAudioSampleAt(targetUs));
            this.audioDecoder.reset();
            this.audioDecoder.configure({
                codec: 'mp4a.40.2',
                sampleRate: this.demuxer.audioSampleRate(),
                numberOfChannels: this.demuxer.audioChannelCount(),
                description: this.demuxer.audioCodecConfig(),
            });
        }

        const speed = this.clock.speed();
        this.clock.reset();
        this.clock.set_speed(0, speed);

        this.updateTime(targetUs / 1000);
        this.updateSubtitles(targetUs);

        this._seekTarget = targetUs;
        this._seekResume = this._seekResumeOverride ?? wasPlaying;
        this._seekResumeOverride = null;
        const config = this.demuxer.codecDescription();
        this.worker.postMessage({ type: 'reset', config });

        // Buffer if streaming
        if (this.demuxer.stillDownloading) {
            this._bufferMore();
        }
    }

    _onSeekReady() {
        const targetUs = this._seekTarget;
        const speed = this.clock.speed();

        this.clock.play(performance.now() - targetUs / 1000 / speed);
        this.clock.pause(performance.now());

        this._seekTarget = null;

        if (this._seekResume) {
            this.frameBuffer.set_skip_until(this.clock.elapsed_us(performance.now()));
            if (this.demuxer.stillDownloading) this._bufferMore();
            this.play();
        } else {
            this._seekDecoding = true;
            this.feedWorker();
            this._seekDecodeCheck();
        }
    }

    async _seekDecodeCheck() {
        if (!this._seekDecoding) return;

        // Update sample count (grows during streaming)
        this.state.set_total_video_samples(this.demuxer.sampleCount());

        // If no samples available at seek position, buffer more
        while (this.demuxer.stillDownloading && this.state.next_video_sample() >= this.state.total_video_samples()) {
            await this._bufferMore();
            this.state.set_total_video_samples(this.demuxer.sampleCount());
        }

        this.feedWorker();

        // Wait for decoded frame
        if (this.frameBuffer.len() > 0) {
            this.frameBuffer.pop_frame(Infinity, true);
            this.renderer.render_current_frame(this.frameBuffer);
            this._seekDecoding = false;
            return;
        }

        requestAnimationFrame(() => this._seekDecodeCheck());
    }

    async _bufferMore() {
        if (this._fetchingData || !this.streamLoader) return;
        this._fetchingData = true;
        try {
            const chunk = await this.streamLoader.fetch_chunk();
            if (chunk.length > 0) {
                // Rust handles format-specific distribution (MKV push or MP4 sample cache)
                this._lastFetchedSample = this.demuxer.pushChunk(chunk, this._lastFetchedSample);
            }

            this.state.set_total_video_samples(this.demuxer.sampleCount());
            this.state.set_total_audio_samples(this.demuxer.audioSampleCount());

            if (this.streamLoader.is_done()) {
                this.demuxer.finish();
                this.demuxer.stillDownloading = false;
                this.state.set_still_downloading(false);
                if (this.demuxer.hasSubtitles()) this.loadSubtitles();
            }
        } finally {
            this._fetchingData = false;
        }
    }

    async bufferAhead(fromVideoSample) {
        if (this._fetchingData || !this.streamLoader) return;
        this._fetchingData = true;
        try {
            const nextIdx = await this.streamLoader.fetchChunk(this.demuxer, fromVideoSample);
            this._lastFetchedSample = Math.max(this._lastFetchedSample, nextIdx);
        } finally {
            this._fetchingData = false;
        }
    }

    // ── Track selection ──

    populateTrackSelectors() {
        const audioSel = this.dom.audioTrackSelect;
        const subSel = this.dom.subTrackSelect;
        if (!audioSel || !subSel) return;

        const audioCount = this.demuxer.audioTrackCount() ?? (this.demuxer.hasAudio() ? 1 : 0);
        if (audioCount > 1) {
            audioSel.innerHTML = '';
            for (let i = 0; i < audioCount; i++) {
                const info = this.demuxer.audioTrackInfo(i);
                const label = info ? `${info.language}${info.name ? ' — ' + info.name : ''}` : `Track ${i + 1}`;
                audioSel.add(new Option(label, i));
            }
            audioSel.style.display = '';
        } else {
            audioSel.style.display = 'none';
        }

        const subCount = this.demuxer.subtitleTrackCount() ?? (this.demuxer.hasSubtitles() ? 1 : 0);
        if (subCount > 0) {
            subSel.innerHTML = '';
            subSel.add(new Option('Subs off', -1));
            for (let i = 0; i < subCount; i++) {
                const info = this.demuxer.subtitleTrackInfo(i);
                const label = info ? `${info.language}${info.name ? ' — ' + info.name : ''}` : `Track ${i + 1}`;
                subSel.add(new Option(label, i));
            }
            subSel.value = '0';
            subSel.style.display = '';
        } else {
            subSel.style.display = 'none';
        }
    }

    switchAudioTrack(index) {
        if (!this.demuxer.setAudioTrack) return;
        this.demuxer.setAudioTrack(index);
        if (this.audioDecoder && this.audioDecoder.state !== 'closed') {
            this.audioBufferQueue = [];
            this.audioDecoder.reset();
            this.audioDecoder.configure({
                codec: 'mp4a.40.2',
                sampleRate: this.demuxer.audioSampleRate(),
                numberOfChannels: this.demuxer.audioChannelCount(),
                description: this.demuxer.audioCodecConfig(),
            });
            this.state.set_next_audio_sample(this.demuxer.findAudioSampleAt(
                this.clock.elapsed_us(performance.now())
            ));
            this.state.set_total_audio_samples(this.demuxer.audioSampleCount());
        }
    }

    switchSubtitleTrack(index) {
        if (index < 0) {
            this.subtitles = [];
            if (this.dom.subtitleEl) this.dom.subtitleEl.innerHTML = '';
            this.lastSubText = '';
            return;
        }
        if (this.demuxer.setSubtitleTrack) this.demuxer.setSubtitleTrack(index);
        this.loadSubtitles();
        this.updateSubtitles(this.clock.elapsed_us(performance.now()));
    }

    // ── Subtitles ──

    loadSubtitles() {
        if (!this.subtitleEngine) {
            this.subtitleEngine = new SubtitleEngine();
        }
        // Add new events since last load
        const count = this.demuxer.subtitleCount();
        for (let i = this.subtitleEngine.count(); i < count; i++) {
            const evt = this.demuxer.subtitleEvent(i);
            if (evt) {
                this.subtitleEngine.add_event(evt.start_us, evt.duration_us, evt.text);
            }
        }
    }

    updateSubtitles(elapsedUs) {
        if (!this.dom.subtitleEl || !this.subtitleEngine || this.subtitleEngine.count() === 0) return;
        const html = this.subtitleEngine.get_active(elapsedUs);
        if (html !== this._lastSubHtml) {
            this.dom.subtitleEl.innerHTML = html;
            this._lastSubHtml = html;
        }
    }

    // ── Audio ──

    async initAudio() {
        this.state.set_total_audio_samples(this.demuxer.audioSampleCount());
        if (this.state.total_audio_samples() === 0) return;

        this.audioCtx = new AudioContext({
            sampleRate: this.demuxer.audioSampleRate(),
        });
        this.audioCtx.suspend();

        const config = {
            codec: 'mp4a.40.2',
            sampleRate: this.demuxer.audioSampleRate(),
            numberOfChannels: this.demuxer.audioChannelCount(),
            description: this.demuxer.audioCodecConfig(),
        };

        const support = await AudioDecoder.isConfigSupported(config);
        if (!support.supported) {
            console.warn('AudioDecoder AAC not supported');
            this.audioCtx = null;
            return;
        }

        this.audioDecoder = new AudioDecoder({
            output: (audioData) => this.onAudioData(audioData),
            error: (e) => console.error('AudioDecoder error:', e),
        });
        this.audioDecoder.configure(config);
        this.state.set_next_audio_sample(0);
    }

    onAudioData(audioData) {
        const numFrames = audioData.numberOfFrames;
        const numChannels = audioData.numberOfChannels;
        const sampleRate = audioData.sampleRate;
        const buf = this.audioCtx.createBuffer(numChannels, numFrames, sampleRate);
        for (let ch = 0; ch < numChannels; ch++) {
            const dest = buf.getChannelData(ch);
            audioData.copyTo(dest, { planeIndex: ch, format: 'f32-planar' });
        }
        const pts = audioData.timestamp;
        this.audioBufferQueue.push({ pts, buffer: buf });
        audioData.close();
    }

    feedAudioDecoder() {
        if (!this.audioDecoder) return;
        while (this.state.next_audio_sample() < this.state.total_audio_samples() &&
               this.audioDecoder.decodeQueueSize < 20) {
            const sample = this.demuxer.readAudioSample(this.state.next_audio_sample());
            if (!sample) break;
            this.state.advance_audio_sample();
            const chunk = new EncodedAudioChunk({
                type: 'key',
                timestamp: sample.timestamp_us,
                duration: sample.duration_us,
                data: sample.data,
            });
            this.audioDecoder.decode(chunk);
        }
    }

    scheduleAudio(elapsedUs) {
        if (!this.audioCtx || this.audioCtx.state !== 'running') return;
        const scheduleAheadUs = 500000;
        const speed = this.clock.speed();
        while (this.audioBufferQueue.length > 0) {
            const { pts, buffer } = this.audioBufferQueue[0];
            if (pts > elapsedUs + scheduleAheadUs) break;
            this.audioBufferQueue.shift();
            if (pts < elapsedUs - 100000) continue;
            const source = this.audioCtx.createBufferSource();
            source.buffer = buffer;
            source.playbackRate.value = speed;
            source.connect(this.audioCtx.destination);
            const when = this.audioAnchorTime +
                (pts - this.audioAnchorElapsed) / speed / 1_000_000;
            const startTime = Math.max(when, this.audioCtx.currentTime);
            if (!isFinite(startTime)) continue;
            source.start(startTime);
        }
    }

    destroy() {
        this.pause();
        this.frameBuffer?.reset();
        if (this.worker) { this.worker.terminate(); this.worker = null; }
        if (this.audioDecoder) { this.audioDecoder.close(); this.audioDecoder = null; }
        if (this.audioCtx) { this.audioCtx.close(); this.audioCtx = null; }
        this.renderer = null;
        this.subtitleEngine = null;
        this.streamLoader = null;
        this.isStreaming = false;
        this._mkvDownloading = false;
        this._fetchingData = false;
    }
}

function fmtTime(sec) {
    const m = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    return `${m}:${s.toString().padStart(2, '0')}`;
}
