import wasmInit, { Demuxer, MkvDemuxer, FrameBuffer, PlaybackClock, Renderer } from './pkg/videoplayer.js';

const dropZone = document.getElementById('drop-zone');
const fileInput = document.getElementById('file-input');
const status = document.getElementById('status');
const playerContainer = document.getElementById('player-container');
const canvas = document.getElementById('canvas');
const playBtn = document.getElementById('play-btn');
const pauseBtn = document.getElementById('pause-btn');
const restartBtn = document.getElementById('restart-btn');
const timeDisplay = document.getElementById('time');
const fpsDisplay = document.getElementById('fps');
const speedSelect = document.getElementById('speed');

let player = null;

class HEVCPlayer {
    constructor(canvas) {
        this.canvas = canvas;
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
        this.audioBufferQueue = []; // { pts, audioBuffer }
        this.audioAnchorTime = 0;    // audioCtx.currentTime at anchor
        this.audioAnchorElapsed = 0; // video elapsed_us at anchor
        // Subtitles
        this.subtitles = [];
        this.subtitleEl = document.getElementById('subtitles');
        this.lastSubText = '';
    }

    async load(arrayBuffer) {
        status.textContent = 'Initializing...';
        await wasmInit();

        this.renderer = new Renderer(this.canvas);
        this.frameBuffer = new FrameBuffer(50, 3);
        this.clock = new PlaybackClock();

        status.textContent = 'Parsing container...';
        const data = new Uint8Array(arrayBuffer);
        // Detect format: MKV starts with 0x1A45DFA3 (EBML), MP4 has 'ftyp' at offset 4
        const isMkv = data[0] === 0x1A && data[1] === 0x45 && data[2] === 0xDF && data[3] === 0xA3;
        this.demuxer = isMkv ? new MkvDemuxer(data) : new Demuxer(data);
        this.isMkv = isMkv;

        this.canvas.width = this.demuxer.width();
        this.canvas.height = this.demuxer.height();
        this.totalSamples = this.demuxer.sample_count();
        this.durationMs = this.demuxer.duration_ms();
        this.nalLengthSize = this.demuxer.nal_length_size();

        status.textContent = `Video: ${this.demuxer.width()}x${this.demuxer.height()}, ${this.totalSamples} frames, ${(this.durationMs / 1000).toFixed(1)}s`;

        // Start decoder worker
        status.textContent += ' — Loading decoder...';
        this.worker = new Worker('./decode-worker.js', { type: 'module' });

        await new Promise((resolve, reject) => {
            const timeout = setTimeout(() => reject(new Error('Decoder init timed out')), 10000);
            this.worker.onmessage = (e) => {
                if (e.data.type === 'ready') {
                    clearTimeout(timeout);
                    resolve();
                }
                if (e.data.type === 'error') console.error('[decoder]', e.data.msg);
                if (e.data.type === 'log') console.log('[decoder]', e.data.msg);
            };
            this.worker.onerror = (e) => { clearTimeout(timeout); reject(e); };
            const hvcc = this.demuxer.codec_description();
            this.worker.postMessage({ type: 'init', codec: 'hevc', config: hvcc });
        });

        // Set up frame handler
        this.worker.onmessage = (e) => this.onWorkerMessage(e.data);

        // Decode first frame as thumbnail
        status.textContent = status.textContent.replace(/ — .*/, '') + ' — Decoding thumbnail...';
        await this.decodeFirstFrame();

        // Reset decoder after thumbnail so playback starts clean
        this.nextSample = 0;
        this.pendingDecodes = 0;
        this.frameBuffer.reset();
        const hvcc2 = this.demuxer.codec_description();
        this.worker.postMessage({ type: 'reset', config: hvcc2 });

        // Init audio if available
        if (this.demuxer.has_audio() && typeof AudioDecoder !== 'undefined') {
            await this.initAudio();
        }

        // Load subtitles if available (MKV only)
        if (this.demuxer.has_subtitles?.()) {
            this.loadSubtitles();
        }

        status.textContent = status.textContent.replace(/ — .*/, '') + ' — Ready';
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
                if (msg.avgMs) console.log(`[perf] ${msg.count} samples, ${msg.frames} frames, avg ${msg.avgMs}ms/sample`);
                if (this.clock.is_playing()) this.feedWorker();
                break;
            case 'flushed':
                this.flushed = true;
                break;
            case 'log':
                console.log('[decoder]', msg.msg);
                break;
            case 'error':
                console.error('[decoder]', msg.msg);
                status.textContent = `Decoder: ${msg.msg}`;
                break;
        }
    }

    feedWorker() {
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
            this.nextSample++;
            if (!sample) continue;
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

        // Pop next displayable frame from Rust buffer
        if (this.frameBuffer.pop_frame(elapsedUs, this.flushed)) {
            this.renderer.render_current_frame(this.frameBuffer);

            // FPS tracking
            this._fpsFrames = (this._fpsFrames || 0) + 1;
            if (!this._fpsTime) this._fpsTime = now;
            if (now - this._fpsTime >= 1000) {
                fpsDisplay.textContent = `${this._fpsFrames} fps`;
                this._fpsFrames = 0;
                this._fpsTime = now;
            }
        }

        this.updateTime(elapsedUs / 1000);

        const done = this.flushed && this.frameBuffer.len() === 0;

        if (done) {
            this.clock.pause(now);
            status.textContent = status.textContent.replace(/ — .*/, '') + ' — Finished';
        } else {
            this.rafId = requestAnimationFrame(() => this.renderLoop());
        }
    }

    updateTime(elapsedMs) {
        const cur = Math.min(elapsedMs / 1000, this.durationMs / 1000);
        const tot = this.durationMs / 1000;
        timeDisplay.textContent = `${fmtTime(cur)} / ${fmtTime(tot)}`;
    }

    setSpeed(speed) {
        // Re-anchor audio before speed change
        if (this.audioCtx && this.audioCtx.state === 'running') {
            this.audioAnchorTime = this.audioCtx.currentTime;
            this.audioAnchorElapsed = this.clock.elapsed_us(performance.now());
        }
        this.clock.set_speed(performance.now(), speed);
    }

    // ── Subtitles ──

    loadSubtitles() {
        const count = this.demuxer.subtitle_count();
        this.subtitles = [];
        for (let i = 0; i < count; i++) {
            const evt = this.demuxer.subtitle_event(i);
            if (!evt) continue;
            // Parse ASS dialogue: strip ASS formatting tags, extract text
            let text = evt.text;
            // ASS dialogue format: Layer,Start,End,Style,Name,MarginL,MarginR,MarginV,Effect,Text
            // In MKV, the timing fields are stripped, just the rest remains
            const parts = text.split(',');
            if (parts.length >= 9) {
                text = parts.slice(8).join(',');
            }
            // Strip ASS override tags like {\b1}, {\an8}, etc.
            text = text.replace(/\{[^}]*\}/g, '');
            // Convert \N to <br>
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
        if (!this.subtitleEl || this.subtitles.length === 0) return;
        // Find active subtitles
        let active = '';
        for (const sub of this.subtitles) {
            if (elapsedUs >= sub.startUs && elapsedUs < sub.endUs) {
                active += (active ? '<br>' : '') + sub.text;
            }
        }
        if (active !== this.lastSubText) {
            this.subtitleEl.innerHTML = active;
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
        // Suspend until play
        this.audioCtx.suspend();

        const config = {
            codec: 'mp4a.40.2', // AAC-LC
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
        // Convert AudioData to AudioBuffer for Web Audio scheduling
        const numFrames = audioData.numberOfFrames;
        const numChannels = audioData.numberOfChannels;
        const sampleRate = audioData.sampleRate;
        const buf = this.audioCtx.createBuffer(numChannels, numFrames, sampleRate);

        for (let ch = 0; ch < numChannels; ch++) {
            const dest = buf.getChannelData(ch);
            audioData.copyTo(dest, { planeIndex: ch, format: 'f32-planar' });
        }

        const pts = audioData.timestamp; // microseconds
        this.audioBufferQueue.push({ pts, buffer: buf });
        audioData.close();
    }

    feedAudioDecoder() {
        if (!this.audioDecoder) return;
        // Feed up to 20 samples ahead
        while (this.nextAudioSample < this.totalAudioSamples &&
               this.audioDecoder.decodeQueueSize < 20) {
            const sample = this.demuxer.read_audio_sample(this.nextAudioSample);
            this.nextAudioSample++;
            if (!sample) continue;
            const chunk = new EncodedAudioChunk({
                type: 'key', // AAC frames are always keyframes
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

            if (pts < elapsedUs - 100000) continue; // skip if too far behind

            const source = this.audioCtx.createBufferSource();
            source.buffer = buffer;
            source.playbackRate.value = speed;
            source.connect(this.audioCtx.destination);

            // Map video PTS to audioCtx.currentTime:
            // At anchor: audioAnchorTime <-> audioAnchorElapsed
            // PTS `pts` is (pts - anchorElapsed) video-microseconds from anchor
            // In real time that's (pts - anchorElapsed) / speed / 1_000_000 seconds
            const when = this.audioAnchorTime +
                (pts - this.audioAnchorElapsed) / speed / 1_000_000;
            source.start(Math.max(when, this.audioCtx.currentTime));
        }
    }

    destroy() {
        this.pause();
        this.frameBuffer.reset();
        if (this.worker) {
            this.worker.terminate();
            this.worker = null;
        }
        if (this.audioDecoder) {
            this.audioDecoder.close();
            this.audioDecoder = null;
        }
        if (this.audioCtx) {
            this.audioCtx.close();
            this.audioCtx = null;
        }
        this.renderer = null;
    }
}

function fmtTime(sec) {
    const m = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    return `${m}:${s.toString().padStart(2, '0')}`;
}

async function loadFile(file) {
    if (player) player.destroy();

    dropZone.classList.add('loading');
    status.textContent = 'Loading file...';
    playerContainer.classList.add('visible');

    try {
        const buf = await file.arrayBuffer();
        player = new HEVCPlayer(canvas);
        await player.load(buf);
        dropZone.classList.remove('loading');
        dropZone.classList.add('hidden');
    } catch (e) {
        dropZone.classList.remove('loading');
        status.textContent = `Error: ${e.message || e}`;
        console.error(e);
    }
}

async function loadUrl(url) {
    if (player) player.destroy();

    dropZone.classList.add('loading');
    status.textContent = 'Fetching file...';
    playerContainer.classList.add('visible');

    try {
        const resp = await fetch(url);
        if (!resp.ok) throw new Error(`Fetch failed: ${resp.status}`);
        const buf = await resp.arrayBuffer();
        player = new HEVCPlayer(canvas);
        await player.load(buf);
        dropZone.classList.remove('loading');
        dropZone.classList.add('hidden');
        document.getElementById('load-sample')?.classList.add('hidden');
    } catch (e) {
        dropZone.classList.remove('loading');
        status.textContent = `Error: ${e.message || e}`;
        console.error(e);
    }
}

document.getElementById('load-sample-mp4')?.addEventListener('click', () => {
    loadUrl('./data/hellmode12_2m.mp4');
});
document.getElementById('load-sample-mkv')?.addEventListener('click', () => {
    loadUrl('./data/hellmode12_2m.mkv');
});

fileInput.addEventListener('change', (e) => {
    if (e.target.files[0]) loadFile(e.target.files[0]);
});

document.addEventListener('dragover', (e) => {
    e.preventDefault();
    if (dropZone) dropZone.classList.add('drag-over');
});
document.addEventListener('dragleave', (e) => {
    if (!e.relatedTarget || e.relatedTarget === document.documentElement) {
        dropZone.classList.remove('drag-over');
    }
});
document.addEventListener('drop', (e) => {
    e.preventDefault();
    dropZone.classList.remove('drag-over');
    const file = e.dataTransfer?.files[0];
    if (file) loadFile(file);
});

playBtn.addEventListener('click', () => player?.play());
pauseBtn.addEventListener('click', () => player?.pause());
restartBtn.addEventListener('click', () => player?.restart());

speedSelect.addEventListener('change', () => {
    player?.setSpeed(parseFloat(speedSelect.value));
});

document.addEventListener('keydown', (e) => {
    if (e.code === 'Space') {
        e.preventDefault();
        if (player?.clock?.is_playing()) player.pause();
        else player?.play();
    }
});
