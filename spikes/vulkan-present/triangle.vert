#version 450
// A full-screen triangle from the vertex index; no vertex buffer.
layout(location = 0) out vec2 uv;
void main() {
    uv = vec2((gl_VertexIndex << 1) & 2, gl_VertexIndex & 2);
    gl_Position = vec4(uv * 2.0 - 1.0, 0.0, 1.0);
}
