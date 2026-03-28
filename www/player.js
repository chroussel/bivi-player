import wasmInit, { Demuxer, FrameBuffer, PlaybackClock, Renderer } from './pkg/videoplayer.js';

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
    }

    async load(arrayBuffer) {
        status.textContent = 'Initializing...';
        await wasmInit();

        this.renderer = new Renderer(this.canvas);
        this.frameBuffer = new FrameBuffer(50, 3);
        this.clock = new PlaybackClock();

        status.textContent = 'Parsing MP4...';
        const data = new Uint8Array(arrayBuffer);
        this.demuxer = new Demuxer(data);

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
        this.feedWorker();
        this.renderLoop();
    }

    pause() {
        this.clock.pause(performance.now());
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

        const hvcc = this.demuxer.codec_description();
        this.worker.postMessage({ type: 'reset', config: hvcc });

        this.renderer?.clear();
        this.updateTime(0);
    }

    renderLoop() {
        if (!this.clock.is_playing()) return;

        this.feedWorker();

        const now = performance.now();
        const elapsedUs = this.clock.elapsed_us(now);

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
        this.clock.set_speed(performance.now(), speed);
    }

    destroy() {
        this.pause();
        this.frameBuffer.reset();
        if (this.worker) {
            this.worker.terminate();
            this.worker = null;
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

document.getElementById('load-sample')?.addEventListener('click', () => {
    loadUrl('./data/hellmode12_2m.mp4');
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
