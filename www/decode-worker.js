import createDecoder from './decoder.js';

let dec = null;
let totalFramesDecoded = 0;

async function init() {
    dec = await createDecoder();
    const ret = dec._decoder_init();
    if (ret !== 0) {
        postMessage({ type: 'error', msg: 'decoder_init failed: ' + ret });
    }
    postMessage({ type: 'log', msg: 'Decoder WASM loaded' });
}

let stridePtr = 0;
function getStridePtr() {
    if (!stridePtr) stridePtr = dec._malloc(4);
    return stridePtr;
}

function pushParameterSets(hvcc) {
    if (!hvcc || hvcc.length < 23) {
        postMessage({ type: 'error', msg: 'hvcC too short: ' + (hvcc ? hvcc.length : 0) });
        return;
    }

    let pos = 22;
    const numArrays = hvcc[pos++];
    let nalCount = 0;

    for (let a = 0; a < numArrays && pos + 3 <= hvcc.length; a++) {
        pos++; // array_completeness | reserved | nal_unit_type
        const numNalus = (hvcc[pos] << 8) | hvcc[pos + 1];
        pos += 2;

        for (let n = 0; n < numNalus && pos + 2 <= hvcc.length; n++) {
            const naluLen = (hvcc[pos] << 8) | hvcc[pos + 1];
            pos += 2;
            if (pos + naluLen > hvcc.length) break;

            const nalData = hvcc.slice(pos, pos + naluLen);
            const ptr = dec._malloc(naluLen);
            dec.HEAPU8.set(nalData, ptr);
            const err = dec._decoder_push_nal(ptr, naluLen, 0n);
            dec._free(ptr);
            if (err !== 0) {
                postMessage({ type: 'log', msg: `param NAL push error: ${err}` });
            }
            nalCount++;
            pos += naluLen;
        }
    }

    // Process parameter sets through the decoder
    let more = 1;
    while (more > 0) {
        more = dec._decoder_decode();
    }

    postMessage({ type: 'log', msg: `Pushed ${nalCount} parameter set NALs (${numArrays} arrays)` });
}

function collectFrames() {
    let count = 0;
    while (dec._decoder_get_next_picture()) {
        const w = dec._decoder_get_width();
        const h = dec._decoder_get_height();
        const pts = Number(dec._decoder_get_picture_pts());

        const bpp = dec._decoder_get_bits_per_pixel();
        const sp = getStridePtr();

        const yPtr = dec._decoder_get_plane(0, sp);
        const yStride = dec.getValue(sp, 'i32');
        const uPtr = dec._decoder_get_plane(1, sp);
        const uStride = dec.getValue(sp, 'i32');
        const vPtr = dec._decoder_get_plane(2, sp);
        const vStride = dec.getValue(sp, 'i32');

        // For 10-bit: convert to 8-bit in worker so main thread gets clean data
        const shift = bpp > 8 ? (bpp - 8) : 0;
        const chromaW = w >> 1, chromaH = h >> 1;

        // Downshift 10→8 bit and strip stride in one pass
        const yData = new Uint8Array(w * h);
        const uData = new Uint8Array(chromaW * chromaH);
        const vData = new Uint8Array(chromaW * chromaH);

        if (shift > 0) {
            // 10-bit: stride is in bytes, samples are uint16 LE
            for (let r = 0; r < h; r++) {
                const rowOff = yPtr + r * yStride;
                const dst = r * w;
                for (let c = 0; c < w; c++) {
                    const lo = dec.HEAPU8[rowOff + c * 2];
                    const hi = dec.HEAPU8[rowOff + c * 2 + 1];
                    yData[dst + c] = ((hi << 8) | lo) >> shift;
                }
            }
            for (let r = 0; r < chromaH; r++) {
                const uOff = uPtr + r * uStride;
                const vOff = vPtr + r * vStride;
                const dst = r * chromaW;
                for (let c = 0; c < chromaW; c++) {
                    uData[dst + c] = ((dec.HEAPU8[uOff + c*2+1] << 8) | dec.HEAPU8[uOff + c*2]) >> shift;
                    vData[dst + c] = ((dec.HEAPU8[vOff + c*2+1] << 8) | dec.HEAPU8[vOff + c*2]) >> shift;
                }
            }
        } else {
            // 8-bit: just strip stride
            for (let r = 0; r < h; r++)
                yData.set(new Uint8Array(dec.HEAPU8.buffer, yPtr + r * yStride, w), r * w);
            for (let r = 0; r < chromaH; r++) {
                uData.set(new Uint8Array(dec.HEAPU8.buffer, uPtr + r * uStride, chromaW), r * chromaW);
                vData.set(new Uint8Array(dec.HEAPU8.buffer, vPtr + r * vStride, chromaW), r * chromaW);
            }
        }

        postMessage({
            type: 'frame', width: w, height: h, pts, bpp: 8,
            yData, uData, vData,
        }, [yData.buffer, uData.buffer, vData.buffer]);
        count++;
        totalFramesDecoded++;
    }
    return count;
}

function decodeSample(data, nalLengthSize, pts) {
    if (!data || data.length === 0) return { frames: 0, ms: 0 };

    const t0 = performance.now();
    const sampleData = data instanceof Uint8Array ? data : new Uint8Array(data);
    const ptr = dec._malloc(sampleData.length);
    dec.HEAPU8.set(sampleData, ptr);
    const ret = dec._decoder_push_mp4_sample(ptr, sampleData.length, nalLengthSize, BigInt(pts));
    dec._free(ptr);

    if (ret !== 0) return { frames: 0, ms: performance.now() - t0 };

    let frames = 0;
    let more = 1;
    while (more > 0) {
        more = dec._decoder_decode();
        if (more < 0) break;
        frames += collectFrames();
    }
    frames += collectFrames();
    return { frames, ms: performance.now() - t0 };
}

function flush() {
    dec._decoder_flush();
    let more = 1;
    while (more > 0) {
        more = dec._decoder_decode();
        collectFrames();
    }
    collectFrames();
    postMessage({ type: 'flushed' });
    postMessage({ type: 'log', msg: `Flush done. Total frames: ${totalFramesDecoded}` });
}

function reset(hvcc) {
    dec._decoder_reset();
    dec._decoder_init();
    totalFramesDecoded = 0;
    if (hvcc) pushParameterSets(hvcc);
}

const ready = init().catch(e => {
    postMessage({ type: 'error', msg: 'init failed: ' + (e.message || e) });
});

onmessage = async (e) => {
    try {
        await ready;
        const msg = e.data;

        switch (msg.type) {
            case 'init':
                try {
                    pushParameterSets(msg.hvcc);
                } catch (e) {
                    postMessage({ type: 'error', msg: 'pushParameterSets: ' + e.message });
                }
                postMessage({ type: 'ready', sharedMemory: dec.HEAPU8.buffer });
                break;

            case 'samples': {
                let totalFrames = 0;
                let totalMs = 0;
                for (const s of msg.samples) {
                    const r = decodeSample(s.data, msg.nalLengthSize, s.pts);
                    totalFrames += r.frames;
                    totalMs += r.ms;
                }
                const avgMs = msg.samples.length > 0 ? (totalMs / msg.samples.length).toFixed(1) : 0;
                postMessage({ type: 'decoded', count: msg.samples.length, frames: totalFrames, avgMs });
                break;
            }

            case 'flush':
                flush();
                break;

            case 'reset':
                reset(msg.hvcc);
                postMessage({ type: 'ready' });
                break;
        }
    } catch (err) {
        postMessage({ type: 'error', msg: 'worker: ' + err.message + '\n' + err.stack });
    }
};
