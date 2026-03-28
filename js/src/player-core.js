import wasmInit, { FrameBuffer, PlaybackClock, Renderer, SubtitleEngine, MediaSession } from './pkg/videoplayer.js';
export { wasmInit };


export class HEVCPlayerCore {
    constructor(canvas, dom) {
        this.canvas = canvas;
        this.dom = dom; // { status, subtitleEl, seekbar, timeDisplay, fpsDisplay, audioTrackSelect, subTrackSelect }
        this.renderer = null;
        this.session = null;
        this.worker = null;
        this.frameBuffer = null;
        this.clock = null;
        this.rafId = null;
        // Audio (browser APIs — must stay JS)
        this.audioCtx = null;
        this.audioDecoder = null;
        this.audioBufferQueue = [];
        this.audioAnchorTime = 0;
        this.audioAnchorElapsed = 0;
        // Streaming
        this._fetchingData = false;
        this._stillDownloading = true;
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

        this._setStatus('Connecting...');
        this._url = url;
        this.session = await new MediaSession(url);

        // Buffer until ready to play
        while (true) {
            await this._bufferMore();
            const frames = this.session.sample_count();
            this._setStatus(`Buffering... ${frames} frames`);
            if (this.session.header_ready() && frames >= 30) break;
            if (!this._stillDownloading) break;
        }

        if (!this.session.header_ready()) throw new Error('Could not parse header');

        this.canvas.width = this.session.width();
        this.canvas.height = this.session.height();
        this._setStatus(`Video: ${this.session.width()}x${this.session.height()}, ${(this.session.duration_ms() / 1000).toFixed(1)}s`);

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
            const config = this.session.codec_description();
            this.worker.postMessage({ type: 'init', codec: 'hevc', config });
        });

        this.worker.onmessage = (e) => this.onWorkerMessage(e.data);
    }

    async _postInit() {
        if (this.session.has_audio() && typeof AudioDecoder !== 'undefined') {
            await this.initAudio();
        }

        if (this.session.has_subtitles()) {
            this.loadSubtitles();
        }

        this.populateTrackSelectors();

        // Feed samples to get a thumbnail frame
        this._wantThumbnail = true;
        this.feedWorker();

        this._setStatus(this._getStatus().replace(/ — .*/, '') + ' — Ready');
    }

    onWorkerMessage(msg) {
        switch (msg.type) {
            case 'frame': {
                this.frameBuffer.push(msg.pts, msg.y, msg.u, msg.v, msg.w, msg.h);
                // Show first frame as thumbnail
                if (this._wantThumbnail && this.frameBuffer.len() > 0) {
                    this.frameBuffer.pop_frame(Infinity, true);
                    this.renderer.render_current_frame(this.frameBuffer);
                    this._wantThumbnail = false;
                }
                break;
            }
            case 'decoded':
                this.session.sub_pending(msg.count);
                if (msg.avgMs && window.verbose) console.log(`[perf] ${msg.count} samples, ${msg.frames} frames, avg ${msg.avgMs}ms/sample`);
                if (this.clock.is_playing() || this._seekDecoding) this.feedWorker();
                break;
            case 'flushed':
                this.session.set_flushed(true);
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
        if (this._seekTarget != null) return;
        if (!this.session.should_feed(this.frameBuffer.len())) {
            if (this.session.should_flush()) {
                this.worker.postMessage({ type: 'flush' });
            }
            return;
        }

        const batchSize = Math.min(10, this.session.total_video_samples() - this.session.next_video_sample());
        const samples = [];

        for (let i = 0; i < batchSize; i++) {
            const sample = this.session.read_sample(this.session.next_video_sample());
            if (!sample) break;
            this.session.advance_video_sample();
            samples.push({
                data: sample.data,
                pts: Math.round(sample.timestamp_us),
            });
        }

        if (samples.length > 0) {
            this.session.add_pending(samples.length);
            this.worker.postMessage(
                { type: 'samples', samples, nalLengthSize: this.session.nal_length_size() }
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
        this.session.set_next_video_sample(0);
        this.session.clear_pending();
        this.session.set_flushed(false);
        this.session.set_next_audio_sample(0);
        this.audioBufferQueue = [];
        this.audioAnchorTime = 0;
        this.audioAnchorElapsed = 0;
        if (this.audioDecoder && this.audioDecoder.state !== 'closed') {
            this.audioDecoder.reset();
            this.audioDecoder.configure({
                codec: 'mp4a.40.2',
                sampleRate: this.session.audio_sample_rate(),
                numberOfChannels: this.session.audio_channel_count(),
                description: this.session.audio_codec_config(),
            });
        }

        const hvcc = this.session.codec_description();
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
        const subCount = this.session.subtitle_count();
        if (subCount > this._lastSubCount) {
            const firstSubs = this._lastSubCount === 0;
            this._lastSubCount = subCount;
            this.loadSubtitles();
            if (firstSubs) this.populateTrackSelectors(); // show dropdown
        }
        this.updateSubtitles(elapsedUs);

        // Buffer ahead
        if (!this._fetchingData && this.session.needs_buffer()) {
            this._bufferMore();
        }

        const MIN_REORDER = 3;
        let frameToShow = null;
        let skipped = 0;
        while (this.decodedFramesCount() > MIN_REORDER || this.session.flushed()) {
            if (!this.frameBuffer.pop_frame(elapsedUs, this.session.flushed())) break;
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

        const done = this.session.flushed() && this.frameBuffer.len() === 0;
        if (done && !this._stillDownloading) {
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
        const cur = Math.min(elapsedMs / 1000, this.session.duration_ms() / 1000);
        const tot = this.session.duration_ms() / 1000;
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
        this.session.set_flushed(false);
        this.session.clear_pending();
        this.session.set_next_video_sample(this.session.find_keyframe_before(targetUs));

        if (this.audioDecoder && this.audioDecoder.state !== 'closed') {
            this.audioBufferQueue = [];
            this.session.set_next_audio_sample(this.session.find_audio_sample_at(targetUs));
            this.audioDecoder.reset();
            this.audioDecoder.configure({
                codec: 'mp4a.40.2',
                sampleRate: this.session.audio_sample_rate(),
                numberOfChannels: this.session.audio_channel_count(),
                description: this.session.audio_codec_config(),
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
        const config = this.session.codec_description();
        this.worker.postMessage({ type: 'reset', config });

        // Buffer if streaming
        if (this._stillDownloading) {
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
            if (this._stillDownloading) this._bufferMore();
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

        // If no samples available at seek position, buffer more
        while (this._stillDownloading && this.session.next_video_sample() >= this.session.total_video_samples()) {
            await this._bufferMore();
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
        if (this._fetchingData) return;
        // Step 1: get fetch range (brief borrow, released immediately)
        const range = this.session.next_fetch_range();
        if (!range) return;
        const [start, end] = range;

        // Step 2: fetch data (no Rust borrow held during await)
        this._fetchingData = true;
        try {
            const resp = await fetch(this._url, {
                headers: { Range: `bytes=${start}-${end - 1}` },
            });
            const data = new Uint8Array(await resp.arrayBuffer());

            // Step 3: push to Rust (brief borrow)
            const more = this.session.push_fetched(data, start);
            this._stillDownloading = more;
            if (!more && this.session.has_subtitles()) {
                this.loadSubtitles();
            }
        } finally {
            this._fetchingData = false;
        }
    }

    // ── Track selection ──

    populateTrackSelectors() {
        const audioSel = this.dom.audioTrackSelect;
        const subSel = this.dom.subTrackSelect;
        if (!audioSel || !subSel) return;

        const audioCount = this.session.audio_track_count() ?? (this.session.has_audio() ? 1 : 0);
        if (audioCount > 1) {
            audioSel.innerHTML = '';
            for (let i = 0; i < audioCount; i++) {
                const info = this.session.audio_track_info(i);
                const label = info ? `${info.language}${info.name ? ' — ' + info.name : ''}` : `Track ${i + 1}`;
                audioSel.add(new Option(label, i));
            }
            audioSel.style.display = '';
        } else {
            audioSel.style.display = 'none';
        }

        const subCount = this.session.subtitle_track_count() ?? (this.session.has_subtitles() ? 1 : 0);
        if (subCount > 0) {
            subSel.innerHTML = '';
            subSel.add(new Option('Subs off', -1));
            for (let i = 0; i < subCount; i++) {
                const info = this.session.subtitle_track_info(i);
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
        if (!this.session.setAudioTrack) return;
        this.session.set_audio_track(index);
        if (this.audioDecoder && this.audioDecoder.state !== 'closed') {
            this.audioBufferQueue = [];
            this.audioDecoder.reset();
            this.audioDecoder.configure({
                codec: 'mp4a.40.2',
                sampleRate: this.session.audio_sample_rate(),
                numberOfChannels: this.session.audio_channel_count(),
                description: this.session.audio_codec_config(),
            });
            this.session.set_next_audio_sample(this.session.find_audio_sample_at(
                this.clock.elapsed_us(performance.now())
            ));
        }
    }

    switchSubtitleTrack(index) {
        if (index < 0) {
            this.subtitles = [];
            if (this.dom.subtitleEl) this.dom.subtitleEl.innerHTML = '';
            this.lastSubText = '';
            return;
        }
        if (this.session.setSubtitleTrack) this.session.set_subtitle_track(index);
        this.loadSubtitles();
        this.updateSubtitles(this.clock.elapsed_us(performance.now()));
    }

    // ── Subtitles ──

    loadSubtitles() {
        if (!this.subtitleEngine) {
            this.subtitleEngine = new SubtitleEngine();
        }
        // Add new events since last load
        const count = this.session.subtitle_count();
        for (let i = this.subtitleEngine.count(); i < count; i++) {
            const evt = this.session.subtitle_event(i);
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
        if (this.session.total_audio_samples() === 0) return;

        this.audioCtx = new AudioContext({
            sampleRate: this.session.audio_sample_rate(),
        });
        this.audioCtx.suspend();

        const config = {
            codec: 'mp4a.40.2',
            sampleRate: this.session.audio_sample_rate(),
            numberOfChannels: this.session.audio_channel_count(),
            description: this.session.audio_codec_config(),
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
        this.session.set_next_audio_sample(0);
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
        while (this.session.next_audio_sample() < this.session.total_audio_samples() &&
               this.audioDecoder.decodeQueueSize < 20) {
            const sample = this.session.read_audio_sample(this.session.next_audio_sample());
            if (!sample) break;
            this.session.advance_audio_sample();
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
        this._fetchingData = false;
    }
}

function fmtTime(sec) {
    const m = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    return `${m}:${s.toString().padStart(2, '0')}`;
}
