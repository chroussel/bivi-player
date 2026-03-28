/**
 * Uniform demuxer interface — wraps Demuxer, MkvDemuxer, StreamingDemuxer, StreamingMkvDemuxer.
 * The player only talks to this interface.
 */
export class DemuxerInterface {
    constructor(inner) {
        this._inner = inner;
        this._stillDownloading = false;
    }

    get stillDownloading() { return this._stillDownloading; }
    set stillDownloading(v) { this._stillDownloading = v; }

    // ── Video ──
    width() { return this._inner.width(); }
    height() { return this._inner.height(); }
    sampleCount() { return this._inner.sample_count(); }
    durationMs() { return this._inner.duration_ms(); }
    codecDescription() { return this._inner.codec_description(); }
    nalLengthSize() { return this._inner.nal_length_size(); }
    readSample(i) { return this._inner.read_sample(i); }
    findKeyframeBefore(us) { return this._inner.find_keyframe_before(us); }

    // ── Audio ──
    hasAudio() { return this._inner.has_audio(); }
    audioSampleRate() { return this._inner.audio_sample_rate(); }
    audioChannelCount() { return this._inner.audio_channel_count(); }
    audioCodecConfig() { return this._inner.audio_codec_config(); }
    audioSampleCount() { return this._inner.audio_sample_count(); }
    readAudioSample(i) { return this._inner.read_audio_sample(i); }
    findAudioSampleAt(us) { return this._inner.find_audio_sample_at(us); }

    // ── Subtitles ──
    hasSubtitles() { return !!this._inner.has_subtitles?.(); }
    subtitleCount() { return this._inner.subtitle_count?.() ?? 0; }
    subtitleEvent(i) { return this._inner.subtitle_event?.(i); }
    subtitleHeader() { return this._inner.subtitle_header?.() ?? ''; }

    // ── Multi-track (MKV) ──
    audioTrackCount() { return this._inner.audio_track_count?.() ?? (this.hasAudio() ? 1 : 0); }
    audioTrackInfo(i) { return this._inner.audio_track_info?.(i); }
    setAudioTrack(i) { this._inner.set_audio_track?.(i); }
    subtitleTrackCount() { return this._inner.subtitle_track_count?.() ?? (this.hasSubtitles() ? 1 : 0); }
    subtitleTrackInfo(i) { return this._inner.subtitle_track_info?.(i); }
    setSubtitleTrack(i) { this._inner.set_subtitle_track?.(i); }

    // ── Streaming MP4 ──
    hasVideoSample(i) { return this._inner.has_video_sample?.(i) ?? true; }
    hasAudioSample(i) { return this._inner.has_audio_sample?.(i) ?? true; }
    videoSampleOffset(i) { return this._inner.video_sample_offset?.(i) ?? 0; }
    videoSampleSize(i) { return this._inner.video_sample_size?.(i) ?? 0; }
    audioSampleOffset(i) { return this._inner.audio_sample_offset?.(i) ?? 0; }
    audioSampleSize(i) { return this._inner.audio_sample_size?.(i) ?? 0; }
    setVideoSampleData(i, d) { this._inner.set_video_sample_data?.(i, d); }
    setAudioSampleData(i, d) { this._inner.set_audio_sample_data?.(i, d); }
    videoBufferRange(s, sec) { return this._inner.video_buffer_range?.(s, sec) ?? [0, 0, s]; }
    evictSamples(vs, ve, as_, ae) { this._inner.evict_samples?.(vs, ve, as_, ae); }

    // ── Streaming MKV ──
    pushData(data) { return this._inner.push_data?.(data) ?? false; }
    headerReady() { return this._inner.header_ready?.() ?? true; }
    finish() { this._inner.finish?.(); }

    /** Should the player flush the decoder? Only when all data is available. */
    canFlush() {
        return !this._stillDownloading;
    }
}
