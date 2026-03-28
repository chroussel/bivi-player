/**
 * <hevc-player> Web Component
 *
 * Usage:
 *   <hevc-player src="/video.mp4"></hevc-player>
 *   <hevc-player></hevc-player>  <!-- file picker mode -->
 */
import { HEVCPlayerCore } from './player-core.js';
import { debounce, accumulatedDebounce } from './debounce.js';

const STYLES = `
:host { display: block; background: #000; position: relative; font-family: system-ui, sans-serif; }
.container { display: flex; flex-direction: column; align-items: center; width: 100%; }
.video-wrap { position: relative; display: inline-block; width: 100%; }
canvas { width: 100%; background: #000; display: block; }
.subtitles {
    position: absolute; bottom: 8%; left: 0; right: 0;
    text-align: center; color: #fff; font-size: 1.4rem;
    text-shadow: 1px 1px 3px #000, -1px -1px 3px #000;
    pointer-events: none; padding: 0 5%;
}
.seekbar { width: 100%; margin: 0; padding: 4px 0; accent-color: #6cf; cursor: pointer; }
.controls {
    display: flex; gap: 0.5rem; align-items: center; padding: 0.4rem;
    width: 100%; flex-wrap: wrap; color: #eee; font-size: 0.85rem;
}
button {
    background: #333; color: #eee; border: 1px solid #555;
    padding: 0.3rem 0.8rem; border-radius: 4px; cursor: pointer; font-size: 0.8rem;
}
button:hover { background: #444; }
select { background: #333; color: #eee; border: 1px solid #555; border-radius: 4px; padding: 0.2rem; font-size: 0.8rem; }
.time { color: #999; }
.fps { color: #6cf; }
.status { color: #999; font-size: 0.85rem; padding: 0.3rem; text-align: center; }
`;

const HTML = `
<div class="container">
    <div class="status" part="status"></div>
    <div class="video-wrap">
        <canvas></canvas>
        <div class="subtitles"></div>
    </div>
    <input type="range" class="seekbar" min="0" max="1000" value="0">
    <div class="controls">
        <button class="play-btn">Play</button>
        <button class="pause-btn">Pause</button>
        <button class="restart-btn">Restart</button>
        <span class="time">0:00 / 0:00</span>
        <select class="speed">
            <option value="0.25">0.25x</option>
            <option value="0.5">0.5x</option>
            <option value="1" selected>1x</option>
            <option value="1.5">1.5x</option>
            <option value="2">2x</option>
        </select>
        <select class="audio-track" style="display:none"></select>
        <select class="sub-track" style="display:none"></select>
        <span class="fps"></span>
    </div>
</div>
`;

export class HevcPlayerElement extends HTMLElement {
    constructor() {
        super();
        this.attachShadow({ mode: 'open' });
        this.shadowRoot.innerHTML = `<style>${STYLES}</style>${HTML}`;

        this._el = {
            canvas: this.shadowRoot.querySelector('canvas'),
            status: this.shadowRoot.querySelector('.status'),
            subtitleEl: this.shadowRoot.querySelector('.subtitles'),
            seekbar: this.shadowRoot.querySelector('.seekbar'),
            timeDisplay: this.shadowRoot.querySelector('.time'),
            fpsDisplay: this.shadowRoot.querySelector('.fps'),
            audioTrackSelect: this.shadowRoot.querySelector('.audio-track'),
            subTrackSelect: this.shadowRoot.querySelector('.sub-track'),
        };
        this._core = null;

        this._bindControls();
        this._bindSeekbar();
        this._bindKeyboard();
    }

    connectedCallback() {
        const src = this.getAttribute('src');
        if (src) this.streamUrl(src);
    }

    static get observedAttributes() { return ['src']; }
    attributeChangedCallback(name, old, val) {
        if (name === 'src' && val && val !== old && this.isConnected) this.streamUrl(val);
    }

    // ── Public API ──

    async loadFile(file) {
        await this._ensureCore();
        try {
            const buf = await file.arrayBuffer();
            await this._core.load(buf);
        } catch (e) { this._el.status.textContent = `Error: ${e.message}`; console.error(e); }
    }

    async loadUrl(url) {
        await this._ensureCore();
        this._el.status.textContent = 'Fetching...';
        try {
            const resp = await fetch(url);
            if (!resp.ok) throw new Error(`${resp.status}`);
            await this._core.load(await resp.arrayBuffer());
        } catch (e) { this._el.status.textContent = `Error: ${e.message}`; console.error(e); }
    }

    async streamUrl(url) {
        // MKV doesn't support Range-based streaming — fall back to full fetch
        if (url.match(/\.mkv(\?|$)/i)) {
            return this.loadUrl(url);
        }
        await this._ensureCore();
        try {
            await this._core.loadStream(url);
        } catch (e) { this._el.status.textContent = `Error: ${e.message}`; console.error(e); }
    }

    play() { this._core?.play(); }
    pause() { this._core?.pause(); }
    seek(timeUs) { this._core?.seek(timeUs); }
    setSpeed(s) { this._core?.setSpeed(s); }

    async _ensureCore() {
        if (this._core) this._core.destroy();
        this._core = new HEVCPlayerCore(this._el.canvas, this._el);
    }

    // ── Event bindings ──

    _bindControls() {
        this.shadowRoot.querySelector('.play-btn').addEventListener('click', () => this.play());
        this.shadowRoot.querySelector('.pause-btn').addEventListener('click', () => this.pause());
        this.shadowRoot.querySelector('.restart-btn').addEventListener('click', () => this._core?.restart());
        this.shadowRoot.querySelector('.speed').addEventListener('change', (e) => this.setSpeed(parseFloat(e.target.value)));
        this.shadowRoot.querySelector('.audio-track').addEventListener('change', (e) => this._core?.switchAudioTrack(parseInt(e.target.value)));
        this.shadowRoot.querySelector('.sub-track').addEventListener('change', (e) => this._core?.switchSubtitleTrack(parseInt(e.target.value)));
    }

    _bindSeekbar() {
        const seekbar = this._el.seekbar;
        const debouncedSeek = debounce((t) => this._core?.seek(t), 100);
        let wasPlaying = false;

        seekbar.addEventListener('mousedown', () => {
            if (!this._core) return;
            this._core._seekDragging = true;
            wasPlaying = this._core.clock?.is_playing();
            if (wasPlaying) this._core.pause();
        });
        seekbar.addEventListener('input', () => {
            if (!this._core) return;
            const t = (seekbar.value / 1000) * this._core.durationMs * 1000;
            this._core.updateTime(t / 1000);
            this._core.updateSubtitles(t);
            debouncedSeek(t);
        });
        seekbar.addEventListener('change', () => {
            if (!this._core) return;
            this._core._seekDragging = false;
            debouncedSeek.cancel();
            const t = (seekbar.value / 1000) * this._core.durationMs * 1000;
            this._core._seekResumeOverride = wasPlaying;
            this._core.seek(t);
        });
    }

    _bindKeyboard() {
        this.setAttribute('tabindex', '0');
        let arrowWasPlaying = false;
        const arrowSeek = accumulatedDebounce((t) => {
            if (!this._core) return;
            this._core._seekResumeOverride = arrowWasPlaying;
            this._core.seek(Math.max(0, Math.min(t, this._core.durationMs * 1000)));
        }, 300);

        this.addEventListener('keydown', (e) => {
            if (e.code === 'Space') {
                e.preventDefault();
                this._core?.clock?.is_playing() ? this.pause() : this.play();
            }
            if ((e.code === 'ArrowRight' || e.code === 'ArrowLeft') && this._core) {
                e.preventDefault();
                const delta = e.code === 'ArrowRight' ? 10_000_000 : -10_000_000;
                if (arrowSeek.pending() == null) {
                    arrowWasPlaying = this._core.clock?.is_playing();
                    if (arrowWasPlaying) this._core.pause();
                }
                arrowSeek.add(delta, () =>
                    this._core._seekTarget ?? this._core.clock?.elapsed_us(performance.now())
                );
                const p = arrowSeek.pending();
                if (p != null) this._core.updateTime(Math.max(0, Math.min(p, this._core.durationMs * 1000)) / 1000);
            }
        });
    }

}

customElements.define('hevc-player', HevcPlayerElement);
