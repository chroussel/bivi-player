const VERTEX_SRC = `
attribute vec2 a_pos;
varying vec2 v_texCoord;
void main() {
    gl_Position = vec4(a_pos, 0.0, 1.0);
    v_texCoord = (a_pos + 1.0) * 0.5;
    v_texCoord.y = 1.0 - v_texCoord.y;
}`;

const FRAGMENT_SRC = `
precision mediump float;
varying vec2 v_texCoord;
uniform sampler2D u_texY;
uniform sampler2D u_texU;
uniform sampler2D u_texV;

void main() {
    float y = texture2D(u_texY, v_texCoord).r;
    float u = texture2D(u_texU, v_texCoord).r;
    float v = texture2D(u_texV, v_texCoord).r;

    // BT.709 TV range
    y = 1.1644 * (y - 0.0625);
    u = u - 0.5;
    v = v - 0.5;

    float r = y + 1.7927 * v;
    float g = y - 0.2132 * u - 0.5329 * v;
    float b = y + 2.1124 * u;

    gl_FragColor = vec4(r, g, b, 1.0);
}`;

export class WebGLRenderer {
    constructor(canvas) {
        const gl = canvas.getContext('webgl');
        if (!gl) throw new Error('WebGL not supported');
        this.gl = gl;
        this.canvas = canvas;

        // Compile shaders
        const vs = this._compile(gl.VERTEX_SHADER, VERTEX_SRC);
        const fs = this._compile(gl.FRAGMENT_SHADER, FRAGMENT_SRC);
        const prog = gl.createProgram();
        gl.attachShader(prog, vs);
        gl.attachShader(prog, fs);
        gl.linkProgram(prog);
        if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
            throw new Error(gl.getProgramInfoLog(prog));
        }
        this.prog = prog;
        gl.useProgram(prog);

        // Full-screen quad
        const buf = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, buf);
        gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1,-1, 1,-1, -1,1, 1,1]), gl.STATIC_DRAW);
        const aPos = gl.getAttribLocation(prog, 'a_pos');
        gl.enableVertexAttribArray(aPos);
        gl.vertexAttribPointer(aPos, 2, gl.FLOAT, false, 0, 0);

        // Create textures
        this.texY = this._createTex(0);
        this.texU = this._createTex(1);
        this.texV = this._createTex(2);

        gl.uniform1i(gl.getUniformLocation(prog, 'u_texY'), 0);
        gl.uniform1i(gl.getUniformLocation(prog, 'u_texU'), 1);
        gl.uniform1i(gl.getUniformLocation(prog, 'u_texV'), 2);
    }

    _compile(type, src) {
        const gl = this.gl;
        const s = gl.createShader(type);
        gl.shaderSource(s, src);
        gl.compileShader(s);
        if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
            throw new Error(gl.getShaderInfoLog(s));
        }
        return s;
    }

    _createTex(unit) {
        const gl = this.gl;
        const tex = gl.createTexture();
        gl.activeTexture(gl.TEXTURE0 + unit);
        gl.bindTexture(gl.TEXTURE_2D, tex);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
        return tex;
    }

    _uploadPlane(unit, tex, data, w, h) {
        const gl = this.gl;
        gl.activeTexture(gl.TEXTURE0 + unit);
        gl.bindTexture(gl.TEXTURE_2D, tex);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.LUMINANCE, w, h, 0, gl.LUMINANCE, gl.UNSIGNED_BYTE, data);
    }

    render(yData, uData, vData, width, height) {
        const gl = this.gl;
        if (this.canvas.width !== width || this.canvas.height !== height) {
            this.canvas.width = width;
            this.canvas.height = height;
        }
        gl.viewport(0, 0, width, height);

        this._uploadPlane(0, this.texY, yData, width, height);
        this._uploadPlane(1, this.texU, uData, width >> 1, height >> 1);
        this._uploadPlane(2, this.texV, vData, width >> 1, height >> 1);

        gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    }

    destroy() {
        const gl = this.gl;
        if (gl) {
            gl.deleteTexture(this.texY);
            gl.deleteTexture(this.texU);
            gl.deleteTexture(this.texV);
            gl.deleteProgram(this.prog);
        }
    }
}
