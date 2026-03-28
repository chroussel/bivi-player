import wasmInit, { Demuxer, MkvDemuxer, StreamingDemuxer, FrameBuffer, PlaybackClock, Renderer } from './pkg/videoplayer.js';
export { wasmInit };
import { StreamLoader } from './stream-loader.js';

export class HEVCPlayerCore {
    constructor(canvas, dom) {
        this.canvas = canvas;
        this.dom = dom; // { status, subtitleEl, seekbar, timeDisplay, fpsDisplay, audioTrackSelect, subTrackSelect }
        this.renderer = null;
        this.demuxer = null;
        this.worker = null;
        this.frameBuffer = null;
        this.clock = null;
        this.nextSample = 0;
        this.totalSamples = 0;
        this.durationMs = 0;
        this.nalLengthSize = 4;
        this.rafId = null;
        this.pendingDecodes = 0;
        this.flushed = false;
        // Audio
        this.audioCtx = null;
        this.audioDecoder = null;
        this.nextAudioSample = 0;
        this.totalAudioSamples = 0;
        this.audioBufferQueue = [];
        this.audioAnchorTime = 0;
        this.audioAnchorElapsed = 0;
        // Streaming
        this.streamLoader = null;
        this.isStreaming = false;
        this._fetchingData = false;
        this._lastFetchedSample = 0;
        // Subtitles
        this.subtitles = [];
        this.lastSubText = '';
    }

    _setStatus(text) {
        if (this.dom.status) this.dom.status.textContent = text;
    }

    _getStatus() {
        return this.dom.status?.textContent || '';
    }

    async load(arrayBuffer) {
        this._setStatus('Initializing...');
        await wasmInit();

        this.renderer = new Renderer(this.canvas);
        this.frameBuffer = new FrameBuffer(50, 3);
        this.clock = new PlaybackClock();

        this._setStatus('Parsing container...');
        const data = new Uint8Array(arrayBuffer);
        const isMkv = data[0] === 0x1A && data[1] === 0x45 && data[2] === 0xDF && data[3] === 0xA3;
        this.demuxer = isMkv ? new MkvDemuxer(data) : new Demuxer(data);
        this.isMkv = isMkv;

        this.canvas.width = this.demuxer.width();
        this.canvas.height = this.demuxer.height();
        this.totalSamples = this.demuxer.sample_count();
        this.durationMs = this.demuxer.duration_ms();
        this.nalLengthSize = this.demuxer.nal_length_size();

        this._setStatus(`Video: ${this.demuxer.width()}x${this.demuxer.height()}, ${this.totalSamples} frames, ${(this.durationMs / 1000).toFixed(1)}s`);

        await this._initDecoder();
        await this._postInit();
    }

    async loadStream(url) {
        this._setStatus('Initializing...');
        await wasmInit();

        this.renderer = new Renderer(this.canvas);
        this.frameBuffer = new FrameBuffer(50, 3);
        this.clock = new PlaybackClock();

        this._setStatus('Fetching headers...');
        this.streamLoader = new StreamLoader(url);
        await this.streamLoader.init();

        this._setStatus('Parsing metadata...');
        this.demuxer = new StreamingDemuxer(this.streamLoader.moovData);
        this.isStreaming = true;

        this.canvas.width = this.demuxer.width();
        this.canvas.height = this.demuxer.height();
        this.totalSamples = this.demuxer.sample_count();
        this.durationMs = this.demuxer.duration_ms();
        this.nalLengthSize = this.demuxer.nal_length_size();

        this._setStatus(`Video: ${this.demuxer.width()}x${this.demuxer.height()}, ${this.totalSamples} frames, ${(this.durationMs / 1000).toFixed(1)}s (streaming)`);

        await this._initDecoder();

        // Buffer initial chunks
        this._lastFetchedSample = 0;
        for (let i = 0; i < 3; i++) {
            await this.bufferAhead(this._lastFetchedSample);
        }

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
            const config = this.demuxer.codec_description();
            this.worker.postMessage({ type: 'init', codec: 'hevc', config });
        });

        this.worker.onmessage = (e) => this.onWorkerMessage(e.data);
    }

    async _postInit() {
        this._setStatus(this._getStatus().replace(/ — .*/, '') + ' — Decoding thumbnail...');
        await this.decodeFirstFrame();

        this.nextSample = 0;
        this.pendingDecodes = 0;
        this.frameBuffer.reset();
        const config = this.demuxer.codec_description();
        this.worker.postMessage({ type: 'reset', config });

        if (this.demuxer.has_audio() && typeof AudioDecoder !== 'undefined') {
            await this.initAudio();
        }

        if (this.demuxer.has_subtitles?.()) {
            this.loadSubtitles();
        }

        this.populateTrackSelectors();

        this._setStatus(this._getStatus().replace(/ — .*/, '') + ' — Ready');
    }

    async decodeFirstFrame() {
        return new Promise((resolve) => {
            const prevHandler = this.worker.onmessage;
            let sent = 0;
            const maxToSend = Math.min(30, this.totalSamples);

            const trySend = () => {
                while (sent < maxToSend && this.nextSample < this.totalSamples) {
                    const sample = this.demuxer.read_sample(this.nextSample);
                    this.nextSample++;
                    if (!sample) continue;
                    this.worker.postMessage({
                        type: 'samples',
                        samples: [{ data: sample.data, pts: 0 }],
                        nalLengthSize: this.nalLengthSize,
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
                    this.pendingDecodes--;
                    if (msg.frames === 0) trySend();
                } else if (msg.type === 'log') {
                    console.log('[decoder]', msg.msg);
                } else if (msg.type === 'error') {
                    console.error('[decoder]', msg.msg);
                }
            };

            this.pendingDecodes = 0;
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
                this.pendingDecodes -= msg.count;
                if (msg.avgMs && window.verbose) console.log(`[perf] ${msg.count} samples, ${msg.frames} frames, avg ${msg.avgMs}ms/sample`);
                if (this.clock.is_playing() || this._seekDecoding) this.feedWorker();
                break;
            case 'flushed':
                this.flushed = true;
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
        if (this.pendingDecodes > 10) return;
        if (this.frameBuffer.len() > 30) return;
        if (this.nextSample >= this.totalSamples) {
            if (!this.flushed && this.pendingDecodes === 0) {
                this.worker.postMessage({ type: 'flush' });
            }
            return;
        }

        const batchSize = Math.min(10, this.totalSamples - this.nextSample);
        const samples = [];

        for (let i = 0; i < batchSize; i++) {
            const sample = this.demuxer.read_sample(this.nextSample);
            if (!sample) break;
            this.nextSample++;
            samples.push({
                data: sample.data,
                pts: Math.round(sample.timestamp_us),
            });
        }

        if (samples.length > 0) {
            this.pendingDecodes += samples.length;
            this.worker.postMessage(
                { type: 'samples', samples, nalLengthSize: this.nalLengthSize }
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
        this.nextSample = 0;
        this.pendingDecodes = 0;
        this.flushed = false;
        this.nextAudioSample = 0;
        this.audioBufferQueue = [];
        this.audioAnchorTime = 0;
        this.audioAnchorElapsed = 0;
        if (this.audioDecoder && this.audioDecoder.state !== 'closed') {
            this.audioDecoder.reset();
            this.audioDecoder.configure({
                codec: 'mp4a.40.2',
                sampleRate: this.demuxer.audio_sample_rate(),
                numberOfChannels: this.demuxer.audio_channel_count(),
                description: this.demuxer.audio_codec_config(),
            });
        }

        const hvcc = this.demuxer.codec_description();
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
        this.updateSubtitles(elapsedUs);

        // Streaming: fetch next chunk when buffer runs low
        if (this.isStreaming && !this._fetchingData) {
            const bufferedAhead = this._lastFetchedSample - this.nextSample;
            if (bufferedAhead < 240 && this._lastFetchedSample < this.totalSamples) {
                this.bufferAhead(this._lastFetchedSample);
            }
        }

        const MIN_REORDER = 3;
        let frameToShow = null;
        let skipped = 0;
        while (this.decodedFramesCount() > MIN_REORDER || this.flushed) {
            if (!this.frameBuffer.pop_frame(elapsedUs, this.flushed)) break;
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

        const done = this.flushed && this.frameBuffer.len() === 0;
        if (done) {
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
        const cur = Math.min(elapsedMs / 1000, this.durationMs / 1000);
        const tot = this.durationMs / 1000;
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
        this.flushed = false;
        this.pendingDecodes = 0;
        this.nextSample = this.demuxer.find_keyframe_before(targetUs);
        this._lastFetchedSample = this.nextSample;

        if (this.audioDecoder && this.audioDecoder.state !== 'closed') {
            this.audioBufferQueue = [];
            this.nextAudioSample = this.demuxer.find_audio_sample_at(targetUs);
            this.audioDecoder.reset();
            this.audioDecoder.configure({
                codec: 'mp4a.40.2',
                sampleRate: this.demuxer.audio_sample_rate(),
                numberOfChannels: this.demuxer.audio_channel_count(),
                description: this.demuxer.audio_codec_config(),
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
        const config = this.demuxer.codec_description();
        this.worker.postMessage({ type: 'reset', config });

        // Buffer if streaming
        if (this.isStreaming) {
            this.bufferAhead(this.nextSample);
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
            this.play();
        } else {
            this._seekDecoding = true;
            this.feedWorker();
            this._seekDecodeCheck();
        }
    }

    _seekDecodeCheck() {
        if (!this._seekDecoding) return;
        if (this.frameBuffer.len() > 0) {
            this.frameBuffer.pop_frame(Infinity, true);
            this.renderer.render_current_frame(this.frameBuffer);
            this._seekDecoding = false;
            return;
        }
        this.feedWorker();
        requestAnimationFrame(() => this._seekDecodeCheck());
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

        const audioCount = this.demuxer.audio_track_count?.() ?? (this.demuxer.has_audio() ? 1 : 0);
        if (audioCount > 1) {
            audioSel.innerHTML = '';
            for (let i = 0; i < audioCount; i++) {
                const info = this.demuxer.audio_track_info?.(i);
                const label = info ? `${info.language}${info.name ? ' — ' + info.name : ''}` : `Track ${i + 1}`;
                audioSel.add(new Option(label, i));
            }
            audioSel.style.display = '';
        } else {
            audioSel.style.display = 'none';
        }

        const subCount = this.demuxer.subtitle_track_count?.() ?? (this.demuxer.has_subtitles?.() ? 1 : 0);
        if (subCount > 0) {
            subSel.innerHTML = '';
            subSel.add(new Option('Subs off', -1));
            for (let i = 0; i < subCount; i++) {
                const info = this.demuxer.subtitle_track_info?.(i);
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
        if (!this.demuxer.set_audio_track) return;
        this.demuxer.set_audio_track(index);
        if (this.audioDecoder && this.audioDecoder.state !== 'closed') {
            this.audioBufferQueue = [];
            this.audioDecoder.reset();
            this.audioDecoder.configure({
                codec: 'mp4a.40.2',
                sampleRate: this.demuxer.audio_sample_rate(),
                numberOfChannels: this.demuxer.audio_channel_count(),
                description: this.demuxer.audio_codec_config(),
            });
            this.nextAudioSample = this.demuxer.find_audio_sample_at(
                this.clock.elapsed_us(performance.now())
            );
            this.totalAudioSamples = this.demuxer.audio_sample_count();
        }
    }

    switchSubtitleTrack(index) {
        if (index < 0) {
            this.subtitles = [];
            if (this.dom.subtitleEl) this.dom.subtitleEl.innerHTML = '';
            this.lastSubText = '';
            return;
        }
        if (this.demuxer.set_subtitle_track) this.demuxer.set_subtitle_track(index);
        this.loadSubtitles();
        this.updateSubtitles(this.clock.elapsed_us(performance.now()));
    }

    // ── Subtitles ──

    loadSubtitles() {
        const count = this.demuxer.subtitle_count();
        this.subtitles = [];
        for (let i = 0; i < count; i++) {
            const evt = this.demuxer.subtitle_event(i);
            if (!evt) continue;
            let text = evt.text;
            const parts = text.split(',');
            if (parts.length >= 9) text = parts.slice(8).join(',');
            text = text.replace(/\{[^}]*\}/g, '');
            text = text.replace(/\\N/g, '<br>');
            text = text.trim();
            if (text) {
                this.subtitles.push({
                    startUs: evt.start_us,
                    endUs: evt.start_us + evt.duration_us,
                    text,
                });
            }
        }
    }

    updateSubtitles(elapsedUs) {
        if (!this.dom.subtitleEl || this.subtitles.length === 0) return;
        let active = '';
        for (const sub of this.subtitles) {
            if (elapsedUs >= sub.startUs && elapsedUs < sub.endUs) {
                active += (active ? '<br>' : '') + sub.text;
            }
        }
        if (active !== this.lastSubText) {
            this.dom.subtitleEl.innerHTML = active;
            this.lastSubText = active;
        }
    }

    // ── Audio ──

    async initAudio() {
        this.totalAudioSamples = this.demuxer.audio_sample_count();
        if (this.totalAudioSamples === 0) return;

        this.audioCtx = new AudioContext({
            sampleRate: this.demuxer.audio_sample_rate(),
        });
        this.audioCtx.suspend();

        const config = {
            codec: 'mp4a.40.2',
            sampleRate: this.demuxer.audio_sample_rate(),
            numberOfChannels: this.demuxer.audio_channel_count(),
            description: this.demuxer.audio_codec_config(),
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
        this.nextAudioSample = 0;
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
        while (this.nextAudioSample < this.totalAudioSamples &&
               this.audioDecoder.decodeQueueSize < 20) {
            const sample = this.demuxer.read_audio_sample(this.nextAudioSample);
            if (!sample) break;
            this.nextAudioSample++;
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
            source.start(Math.max(when, this.audioCtx.currentTime));
        }
    }

    destroy() {
        this.pause();
        this.frameBuffer?.reset();
        if (this.worker) { this.worker.terminate(); this.worker = null; }
        if (this.audioDecoder) { this.audioDecoder.close(); this.audioDecoder = null; }
        if (this.audioCtx) { this.audioCtx.close(); this.audioCtx = null; }
        this.renderer = null;
    }
}

function fmtTime(sec) {
    const m = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    return `${m}:${s.toString().padStart(2, '0')}`;
}
