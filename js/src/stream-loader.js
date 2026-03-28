/**
 * Stream loader — fetches MP4 data via Range requests.
 * Loads moov box first, then fetches media samples on demand.
 */
export class StreamLoader {
    constructor(url) {
        this.url = url;
        this.fileSize = 0;
        this.moovData = null;
    }

    async initHead() {
        const head = await fetch(this.url, { method: 'HEAD' });
        this.fileSize = parseInt(head.headers.get('Content-Length') || '0');
        if (!this.fileSize) throw new Error('Cannot determine file size');
    }

    async init() {
        await this.initHead();

        // Fetch first 64KB — should contain ftyp + moov for faststart files
        let moovData = await this.fetchRange(0, Math.min(65536, this.fileSize));

        // Find moov box — scan top-level boxes
        let moovInfo = this.findBox(moovData, 'moov');

        if (!moovInfo) {
            // Non-faststart: scan top-level box headers to find moov file offset.
            // Box headers are in our initial fetch — we skip large boxes by size.
            let filePos = 0;
            let moovFileOffset = -1;
            const view = new DataView(moovData.buffer, moovData.byteOffset, moovData.byteLength);

            while (filePos + 8 <= this.fileSize) {
                const hdr = await this.readBoxHeader(filePos, moovData);
                if (!hdr) break;
                if (hdr.type === 'moov') { moovFileOffset = filePos; break; }
                filePos += hdr.size;
            }

            if (moovFileOffset < 0) {
                // Headers didn't reveal moov — it might be beyond initial fetch.
                // Compute moov offset from file size: fetch last 10MB as fallback.
                const tailSize = Math.min(10 * 1024 * 1024, this.fileSize);
                const tailStart = this.fileSize - tailSize;
                const tailData = await this.fetchRange(tailStart, this.fileSize);
                const moovTail = this.findBox(tailData, 'moov');
                if (!moovTail) throw new Error('Cannot find moov box');
                this.moovData = tailData.slice(moovTail.contentStart, moovTail.contentStart + moovTail.contentSize);
                return;
            }

            // Fetch moov box at known offset
            const hdrData = await this.fetchRange(moovFileOffset, moovFileOffset + 16);
            const hView = new DataView(hdrData.buffer);
            let moovSize = hView.getUint32(0);
            if (moovSize === 1) {
                moovSize = hView.getUint32(8) * 0x100000000 + hView.getUint32(12);
            }
            const fullMoov = await this.fetchRange(moovFileOffset, moovFileOffset + moovSize);
            this.moovData = fullMoov.slice(8, moovSize);
            return;
        }

        // moov might extend beyond our initial fetch
        const moovEnd = moovInfo.boxStart + moovInfo.boxSize;
        if (moovEnd > moovData.byteLength) {
            moovData = await this.fetchRange(0, moovEnd);
            moovInfo = this.findBox(moovData, 'moov');
        }

        this.moovData = new Uint8Array(moovData.buffer, moovInfo.contentStart, moovInfo.contentSize);
    }

    /**
     * Read an 8-byte box header at an absolute file position.
     * Uses `cached` if the position is within it, otherwise fetches.
     */
    async readBoxHeader(filePos, cached) {
        let bytes;
        if (filePos + 16 <= cached.byteLength) {
            bytes = cached.slice(filePos, filePos + 16);
        } else {
            bytes = await this.fetchRange(filePos, Math.min(filePos + 16, this.fileSize));
        }
        if (bytes.byteLength < 8) return null;
        const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
        let size = view.getUint32(0);
        const type = String.fromCharCode(bytes[4], bytes[5], bytes[6], bytes[7]);
        if (size === 1 && bytes.byteLength >= 16) {
            size = view.getUint32(8) * 0x100000000 + view.getUint32(12);
        }
        if (size < 8) return null;
        return { type, size };
    }

    findBox(data, type) {
        const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
        let pos = 0;
        while (pos + 8 <= data.byteLength) {
            const size = view.getUint32(pos);
            const boxType = String.fromCharCode(data[pos+4], data[pos+5], data[pos+6], data[pos+7]);
            if (size < 8) break;
            if (boxType === type) {
                return {
                    boxStart: pos,
                    boxSize: size,
                    contentStart: pos + 8,
                    contentSize: size - 8,
                };
            }
            pos += size;
        }
        return null;
    }

    async fetchRange(start, end) {
        const resp = await fetch(this.url, {
            headers: { Range: `bytes=${start}-${end - 1}` },
        });
        if (!resp.ok && resp.status !== 206) {
            throw new Error(`Range fetch failed: ${resp.status}`);
        }
        return new Uint8Array(await resp.arrayBuffer());
    }

    /**
     * Fetch ~1MB chunk of interleaved data starting from a video sample.
     * Distributes fetched bytes to both video and audio sample caches.
     */
    async fetchChunk(demuxer, fromVideoSample) {
        const CHUNK_SIZE = 1024 * 1024; // 1MB

        // Find the file offset of the first uncached video sample
        let startOff = demuxer.video_sample_offset(fromVideoSample);
        if (startOff <= 0) return;

        // Expand start backward slightly to catch interleaved audio before the video sample
        startOff = Math.max(0, startOff - 64 * 1024); // 64KB before
        const endOff = Math.min(startOff + CHUNK_SIZE, this.fileSize);

        const data = await this.fetchRange(startOff, endOff);

        // Distribute to video samples that fall within this range
        const vCount = demuxer.sample_count();
        let lastVideoIdx = fromVideoSample;
        for (let i = fromVideoSample; i < vCount; i++) {
            const off = demuxer.video_sample_offset(i);
            const sz = demuxer.video_sample_size(i);
            if (off + sz > endOff) break;
            if (off < startOff || sz === 0) continue;
            if (demuxer.has_video_sample(i)) continue;
            demuxer.set_video_sample_data(i, data.slice(off - startOff, off - startOff + sz));
            lastVideoIdx = i;
        }

        // Distribute to audio samples that fall within this range
        // Use binary-ish search: find audio sample near our time, scan around it
        if (demuxer.has_audio()) {
            const aCount = demuxer.audio_sample_count();
            // Find first audio sample near the current video sample's timestamp
            const vSample = demuxer.read_sample(fromVideoSample);
            const pts = vSample ? vSample.timestamp_us : 0;
            let aStart = demuxer.find_audio_sample_at(pts);
            // Scan backward to catch any before startOff
            aStart = Math.max(0, aStart - 50);
            for (let i = aStart; i < aCount; i++) {
                const off = demuxer.audio_sample_offset(i);
                if (off >= endOff) break;
                if (off < startOff) continue;
                const sz = demuxer.audio_sample_size(i);
                if (sz === 0 || off + sz > endOff) continue;
                if (demuxer.has_audio_sample(i)) continue;
                demuxer.set_audio_sample_data(i, data.slice(off - startOff, off - startOff + sz));
            }
        }

        // Evict old samples (keep 30s behind)
        const keepStart = Math.max(0, fromVideoSample - 720);
        demuxer.evict_samples(keepStart, lastVideoIdx + 1, 0, demuxer.audio_sample_count());

        return lastVideoIdx + 1; // next sample to fetch
    }

    /**
     * Fetch sample data for a range of video/audio samples.
     * Fetches one big byte range covering all samples, then distributes.
     */
    async fetchSamples(demuxer, type, startIdx, endIdx) {
        // Find overall byte range spanning all needed samples
        let minOff = Infinity, maxEnd = 0;
        const samples = [];
        for (let i = startIdx; i < endIdx; i++) {
            const hasIt = type === 'video'
                ? demuxer.has_video_sample(i)
                : demuxer.has_audio_sample(i);
            if (hasIt) continue;

            const offset = type === 'video'
                ? demuxer.video_sample_offset(i)
                : demuxer.audio_sample_offset(i);
            const size = type === 'video'
                ? demuxer.video_sample_size(i)
                : demuxer.audio_sample_size(i);
            if (size === 0) continue;
            samples.push({ idx: i, offset, size });
            minOff = Math.min(minOff, offset);
            maxEnd = Math.max(maxEnd, offset + size);
        }

        if (samples.length === 0) return;

        // Single fetch for the entire range
        const data = await this.fetchRange(minOff, maxEnd);

        // Distribute to individual samples
        for (const s of samples) {
            const localOff = s.offset - minOff;
            const sampleData = data.slice(localOff, localOff + s.size);
            if (type === 'video') {
                demuxer.set_video_sample_data(s.idx, sampleData);
            } else {
                demuxer.set_audio_sample_data(s.idx, sampleData);
            }
        }
    }
}
