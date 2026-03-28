//! StreamLoader — fetches media data via Range requests from Rust/WASM.
//! Auto-detects format, finds moov for MP4, progressive chunks for MKV.

use js_sys::{ArrayBuffer, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Headers, Request, RequestInit, Response};

use crate::format_detect::{detect_format, ContainerFormat};

async fn fetch_url(url: &str, headers: Option<&Headers>) -> Result<Response, JsValue> {
    let mut opts = RequestInit::new();
    opts.method("GET");
    if let Some(h) = headers {
        opts.headers(h);
    }
    let request = Request::new_with_str_and_init(url, &opts)?;
    let window = web_sys::window().ok_or("no window")?;
    let resp_val = JsFuture::from(window.fetch_with_request(&request)).await?;
    Ok(resp_val.dyn_into()?)
}

async fn fetch_head(url: &str) -> Result<(u64, Response), JsValue> {
    let mut opts = RequestInit::new();
    opts.method("HEAD");
    let request = Request::new_with_str_and_init(url, &opts)?;
    let window = web_sys::window().ok_or("no window")?;
    let resp_val = JsFuture::from(window.fetch_with_request(&request)).await?;
    let resp: Response = resp_val.dyn_into()?;
    let size = resp.headers()
        .get("Content-Length")?.unwrap_or_default()
        .parse::<u64>().unwrap_or(0);
    Ok((size, resp))
}

async fn fetch_range(url: &str, start: u64, end: u64) -> Result<Vec<u8>, JsValue> {
    let headers = Headers::new()?;
    headers.set("Range", &format!("bytes={}-{}", start, end - 1))?;
    let resp = fetch_url(url, Some(&headers)).await?;
    let buf: ArrayBuffer = JsFuture::from(resp.array_buffer()?).await?.dyn_into()?;
    let arr = Uint8Array::new(&buf);
    Ok(arr.to_vec())
}

#[wasm_bindgen]
pub struct StreamLoader {
    url: String,
    file_size: u64,
    format: ContainerFormat,
    init_data: Vec<u8>,
    mkv_offset: u64,
    done: bool,
    last_fetched_sample: u32,
}

#[wasm_bindgen]
impl StreamLoader {
    /// Create and initialize a StreamLoader.
    /// Fetches HEAD + first 64KB, detects format, finds moov if MP4.
    #[wasm_bindgen(constructor)]
    pub async fn new(url: String) -> Result<StreamLoader, JsValue> {
        console_error_panic_hook::set_once();

        let (file_size, _) = fetch_head(&url).await?;
        if file_size == 0 {
            return Err("Cannot determine file size".into());
        }

        // Probe first 64KB
        let probe_size = (file_size as usize).min(65536);
        let probe_data = fetch_range(&url, 0, probe_size as u64).await?;
        let format = detect_format(&probe_data);

        let mut loader = StreamLoader {
            url,
            file_size,
            format,
            init_data: Vec::new(),
            mkv_offset: probe_size as u64,
            done: false,
            last_fetched_sample: 0,
        };

        match format {
            ContainerFormat::Mkv => {
                // MKV: probe data will be pushed on first buffer_more
                loader.init_data = probe_data; // reuse as initial MKV data
            }
            ContainerFormat::Mp4 => {
                // MP4: find moov box
                loader.find_moov(&probe_data).await?;
            }
            _ => return Err("Unknown container format".into()),
        }

        Ok(loader)
    }

    async fn find_moov(&mut self, probe: &[u8]) -> Result<(), JsValue> {
        // Scan for moov in probe data
        let mut pos = 0usize;
        while pos + 8 <= probe.len() {
            let size = u32::from_be_bytes([probe[pos], probe[pos+1], probe[pos+2], probe[pos+3]]) as u64;
            let box_type = &probe[pos+4..pos+8];
            if size < 8 { break; }

            if box_type == b"moov" {
                // moov found — check if it fits in probe
                let moov_end = pos as u64 + size;
                if moov_end <= probe.len() as u64 {
                    self.init_data = probe[pos+8..moov_end as usize].to_vec();
                } else {
                    // Need to fetch the rest
                    let full = fetch_range(&self.url, pos as u64, moov_end).await?;
                    self.init_data = full[8..].to_vec();
                }
                return Ok(());
            }
            pos += size as usize;
        }

        // moov not in probe — it's after mdat. Fetch header at computed offset.
        if pos as u64 <= self.file_size {
            let hdr = fetch_range(&self.url, pos as u64, (pos as u64) + 16).await?;
            if hdr.len() >= 8 && &hdr[4..8] == b"moov" {
                let moov_size = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as u64;
                let full = fetch_range(&self.url, pos as u64, pos as u64 + moov_size).await?;
                self.init_data = full[8..].to_vec();
                return Ok(());
            }
        }

        Err("Cannot find moov box".into())
    }

    // ── Public API ──

    pub fn format(&self) -> ContainerFormat { self.format }
    pub fn is_mkv(&self) -> bool { self.format == ContainerFormat::Mkv }
    pub fn file_size(&self) -> f64 { self.file_size as f64 }
    pub fn is_done(&self) -> bool { self.done }
    pub fn init_data(&self) -> Vec<u8> { self.init_data.clone() }

    /// Fetch next 1MB of data. For MKV: returns raw chunk. For MP4: distributes to demuxer.
    pub async fn fetch_chunk(&mut self) -> Result<Vec<u8>, JsValue> {
        let chunk_size = 1024 * 1024u64;
        let start = self.mkv_offset;
        let end = (start + chunk_size).min(self.file_size);
        if start >= self.file_size {
            self.done = true;
            return Ok(Vec::new());
        }
        let data = fetch_range(&self.url, start, end).await?;
        self.mkv_offset = end;
        if self.mkv_offset >= self.file_size {
            self.done = true;
        }
        Ok(data)
    }

    /// Fetch 1MB starting from a specific byte offset.
    pub async fn fetch_range_at(&mut self, offset: u64) -> Result<Vec<u8>, JsValue> {
        let chunk_size = 1024 * 1024u64;
        let start = offset;
        let end = (start + chunk_size).min(self.file_size);
        if start >= self.file_size {
            self.done = true;
            return Ok(Vec::new());
        }
        fetch_range(&self.url, start, end).await
    }

    /// Fetch a specific byte range.
    pub async fn fetch_range_bytes(&self, start: f64, end: f64) -> Result<Vec<u8>, JsValue> {
        fetch_range(&self.url, start as u64, end as u64).await
    }
}
