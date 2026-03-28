/* Generic codec worker — loads emscripten codec WASM implementing codec_api.h */

const CODECS = {
    hevc: () => import('./codec-hevc.js'),
    // h264: () => import('./codec-h264.js'),
    // av1:  () => import('./codec-av1.js'),
};

let dec = null;
let frameStructSize = 0;

async function init(codecName) {
    const loader = CODECS[codecName];
    if (!loader) throw new Error(`Unknown codec: ${codecName}`);
    const mod = await loader();
    dec = await mod.default();
    dec._codec_init();
    frameStructSize = dec._codec_frame_size();
}

function configure(configData) {
    const ptr = dec._malloc(configData.length);
    dec.HEAPU8.set(configData, ptr);
    dec._codec_configure(ptr, configData.length);
    dec._free(ptr);
    let more = 1;
    while (more > 0) more = dec._codec_decode();
}

function collectFrames() {
    const count = dec._codec_collect_frames();
    if (count === 0) return 0;
    const base = dec._codec_get_frames();
    for (let i = 0; i < count; i++) {
        const off = base + i * frameStructSize;
        const dv = new DataView(dec.HEAPU8.buffer, off, frameStructSize);
        const w = dv.getInt32(0, true);
        const h = dv.getInt32(4, true);
        const yPtr = dv.getUint32(24, true);
        const yStride = dv.getInt32(28, true);
        const uPtr = dv.getUint32(32, true);
        const uStride = dv.getInt32(36, true);
        const vPtr = dv.getUint32(40, true);
        const vStride = dv.getInt32(44, true);
        // Data is already 8-bit, stride-stripped by C wrapper
        const cw = w >> 1, ch = h >> 1;
        const y = new Uint8Array(dec.HEAPU8.buffer, yPtr, w * h).slice();
        const u = new Uint8Array(dec.HEAPU8.buffer, uPtr, cw * ch).slice();
        const v = new Uint8Array(dec.HEAPU8.buffer, vPtr, cw * ch).slice();
        postMessage({
            type: 'frame',
            pts: Number(dv.getBigInt64(16, true)),
            w, h, y, u, v,
        }, [y.buffer, u.buffer, v.buffer]);
    }
    return count;
}

function decodeSample(data, nalLengthSize, pts) {
    const t0 = performance.now();
    const buf = data instanceof Uint8Array ? data : new Uint8Array(data);
    const ptr = dec._malloc(buf.length);
    dec.HEAPU8.set(buf, ptr);
    dec._codec_push_sample(ptr, buf.length, nalLengthSize, BigInt(pts));
    dec._free(ptr);
    let frames = 0, more = 1;
    while (more > 0) {
        more = dec._codec_decode();
        if (more < 0) break;
        frames += collectFrames();
    }
    frames += collectFrames();
    return { frames, ms: performance.now() - t0 };
}

onmessage = async (e) => {
    try {
        const msg = e.data;
        switch (msg.type) {
            case 'init':
                await init(msg.codec);
                configure(msg.config);
                postMessage({ type: 'ready' });
                break;
            case 'samples': {
                let frames = 0, ms = 0;
                for (const s of msg.samples) {
                    const r = decodeSample(s.data, msg.nalLengthSize, s.pts);
                    frames += r.frames;
                    ms += r.ms;
                }
                const avgMs = msg.samples.length > 0 ? (ms / msg.samples.length).toFixed(1) : 0;
                postMessage({ type: 'decoded', count: msg.samples.length, frames, avgMs });
                break;
            }
            case 'flush':
                dec._codec_flush();
                let more = 1;
                while (more > 0) { more = dec._codec_decode(); collectFrames(); }
                collectFrames();
                postMessage({ type: 'flushed' });
                break;
            case 'reset':
                dec._codec_reset();
                dec._codec_init();
                if (msg.config) configure(msg.config);
                postMessage({ type: 'ready' });
                break;
        }
    } catch (err) {
        postMessage({ type: 'error', msg: err.message + '\n' + err.stack });
    }
};
