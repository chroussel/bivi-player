#![allow(non_camel_case_types)]

use core::ffi::c_void;

pub type de265_decoder_context = c_void;
pub type de265_image = c_void;
pub type de265_error = i32;
pub type de265_PTS = i64;

pub const DE265_OK: de265_error = 0;
pub const DE265_ERROR_WAITING_FOR_INPUT_DATA: de265_error = 4;

pub const DE265_CHROMA_420: i32 = 1;

extern "C" {
    pub fn de265_new_decoder() -> *mut de265_decoder_context;
    pub fn de265_free_decoder(ctx: *mut de265_decoder_context) -> de265_error;
    pub fn de265_reset(ctx: *mut de265_decoder_context);
    pub fn de265_start_worker_threads(ctx: *mut de265_decoder_context, n: i32) -> de265_error;

    pub fn de265_push_data(
        ctx: *mut de265_decoder_context,
        data: *const u8,
        length: i32,
        pts: de265_PTS,
        user_data: *mut c_void,
    ) -> de265_error;
    pub fn de265_push_end_of_NAL(ctx: *mut de265_decoder_context);
    pub fn de265_push_end_of_frame(ctx: *mut de265_decoder_context);
    pub fn de265_flush_data(ctx: *mut de265_decoder_context) -> de265_error;

    pub fn de265_decode(ctx: *mut de265_decoder_context, more: *mut i32) -> de265_error;
    pub fn de265_get_next_picture(ctx: *mut de265_decoder_context) -> *const de265_image;

    pub fn de265_get_image_width(img: *const de265_image, channel: i32) -> i32;
    pub fn de265_get_image_height(img: *const de265_image, channel: i32) -> i32;
    pub fn de265_get_bits_per_pixel(img: *const de265_image, channel: i32) -> i32;
    pub fn de265_get_image_PTS(img: *const de265_image) -> de265_PTS;
    pub fn de265_get_chroma_format(img: *const de265_image) -> i32;
    pub fn de265_get_image_plane(
        img: *const de265_image,
        channel: i32,
        out_stride: *mut i32,
    ) -> *const u8;
}
