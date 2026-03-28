use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlCanvasElement, WebGlProgram, WebGlRenderingContext as GL, WebGlShader, WebGlTexture};

use crate::frame_buffer::FrameBuffer;

const VERT_SRC: &str = r#"
attribute vec2 a_pos;
varying vec2 v_uv;
void main() {
    gl_Position = vec4(a_pos, 0.0, 1.0);
    v_uv = vec2((a_pos.x + 1.0) * 0.5, 1.0 - (a_pos.y + 1.0) * 0.5);
}"#;

const FRAG_SRC: &str = r#"
precision mediump float;
varying vec2 v_uv;
uniform sampler2D u_y, u_u, u_v;
void main() {
    float y = texture2D(u_y, v_uv).r;
    float u = texture2D(u_u, v_uv).r - 0.5;
    float v = texture2D(u_v, v_uv).r - 0.5;
    y = 1.1644 * (y - 0.0625);
    gl_FragColor = vec4(
        y + 1.7927 * v,
        y - 0.2132 * u - 0.5329 * v,
        y + 2.1124 * u,
        1.0
    );
}"#;

fn compile_shader(gl: &GL, kind: u32, src: &str) -> Result<WebGlShader, String> {
    let s = gl.create_shader(kind).ok_or("create_shader failed")?;
    gl.shader_source(&s, src);
    gl.compile_shader(&s);
    if !gl
        .get_shader_parameter(&s, GL::COMPILE_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        return Err(gl.get_shader_info_log(&s).unwrap_or_default());
    }
    Ok(s)
}

fn create_tex(gl: &GL, unit: u32) -> WebGlTexture {
    let tex = gl.create_texture().unwrap();
    gl.active_texture(GL::TEXTURE0 + unit);
    gl.bind_texture(GL::TEXTURE_2D, Some(&tex));
    gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MIN_FILTER, GL::LINEAR as i32);
    gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MAG_FILTER, GL::LINEAR as i32);
    gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_WRAP_S, GL::CLAMP_TO_EDGE as i32);
    gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_WRAP_T, GL::CLAMP_TO_EDGE as i32);
    tex
}

#[wasm_bindgen]
pub struct Renderer {
    gl: GL,
    canvas: HtmlCanvasElement,
    #[allow(dead_code)]
    program: WebGlProgram,
    tex_y: WebGlTexture,
    tex_u: WebGlTexture,
    tex_v: WebGlTexture,
    cur_w: u32,
    cur_h: u32,
}

#[wasm_bindgen]
impl Renderer {
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: HtmlCanvasElement) -> Result<Renderer, JsValue> {
        let gl: GL = canvas
            .get_context("webgl")?
            .ok_or("no webgl")?
            .dyn_into()?;

        let vs =
            compile_shader(&gl, GL::VERTEX_SHADER, VERT_SRC).map_err(|e| JsValue::from_str(&e))?;
        let fs = compile_shader(&gl, GL::FRAGMENT_SHADER, FRAG_SRC)
            .map_err(|e| JsValue::from_str(&e))?;

        let program = gl.create_program().ok_or("create_program")?;
        gl.attach_shader(&program, &vs);
        gl.attach_shader(&program, &fs);
        gl.link_program(&program);
        if !gl
            .get_program_parameter(&program, GL::LINK_STATUS)
            .as_bool()
            .unwrap_or(false)
        {
            return Err(JsValue::from_str(
                &gl.get_program_info_log(&program).unwrap_or_default(),
            ));
        }
        gl.use_program(Some(&program));

        let buf = gl.create_buffer().ok_or("create_buffer")?;
        gl.bind_buffer(GL::ARRAY_BUFFER, Some(&buf));
        let verts: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
        unsafe {
            let view = js_sys::Float32Array::view(&verts);
            gl.buffer_data_with_array_buffer_view(GL::ARRAY_BUFFER, &view, GL::STATIC_DRAW);
        }
        let a_pos = gl.get_attrib_location(&program, "a_pos") as u32;
        gl.enable_vertex_attrib_array(a_pos);
        gl.vertex_attrib_pointer_with_i32(a_pos, 2, GL::FLOAT, false, 0, 0);

        let tex_y = create_tex(&gl, 0);
        let tex_u = create_tex(&gl, 1);
        let tex_v = create_tex(&gl, 2);

        gl.uniform1i(gl.get_uniform_location(&program, "u_y").as_ref(), 0);
        gl.uniform1i(gl.get_uniform_location(&program, "u_u").as_ref(), 1);
        gl.uniform1i(gl.get_uniform_location(&program, "u_v").as_ref(), 2);

        Ok(Renderer {
            gl,
            canvas,
            program,
            tex_y,
            tex_u,
            tex_v,
            cur_w: 0,
            cur_h: 0,
        })
    }

    fn upload_plane(&self, unit: u32, tex: &WebGlTexture, data: &[u8], w: i32, h: i32) {
        let gl = &self.gl;
        gl.active_texture(GL::TEXTURE0 + unit);
        gl.bind_texture(GL::TEXTURE_2D, Some(tex));
        gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
            GL::TEXTURE_2D,
            0,
            GL::LUMINANCE as i32,
            w,
            h,
            0,
            GL::LUMINANCE,
            GL::UNSIGNED_BYTE,
            Some(data),
        )
        .ok();
    }

    pub fn render(&mut self, y: &[u8], u: &[u8], v: &[u8], width: u32, height: u32) {
        if width != self.cur_w || height != self.cur_h {
            self.canvas.set_width(width);
            self.canvas.set_height(height);
            self.cur_w = width;
            self.cur_h = height;
        }
        self.gl.viewport(0, 0, width as i32, height as i32);
        let w = width as i32;
        let h = height as i32;
        self.upload_plane(0, &self.tex_y.clone(), y, w, h);
        self.upload_plane(1, &self.tex_u.clone(), u, w >> 1, h >> 1);
        self.upload_plane(2, &self.tex_v.clone(), v, w >> 1, h >> 1);
        self.gl.draw_arrays(GL::TRIANGLE_STRIP, 0, 4);
    }

    pub fn clear(&self) {
        self.gl.clear_color(0.0, 0.0, 0.0, 1.0);
        self.gl.clear(GL::COLOR_BUFFER_BIT);
    }

    /// Render directly from FrameBuffer's current frame — avoids extra copies.
    pub fn render_current_frame(&mut self, fb: &FrameBuffer) {
        if let Some(f) = &fb.current {
            self.render(&f.y, &f.u, &f.v, f.width, f.height);
        }
    }
}
