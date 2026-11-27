use serde::Serialize;

/// The part of the host a library belongs to. It decides which layer of cs2metal
/// answers for it: the glibc ABI layer, a guest-loaded runtime, or a substitution
/// by a macOS library with the same C API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// libc, libm, libpthread, libdl, librt and the dynamic loader itself.
    Glibc,
    /// libstdc++ and libgcc_s.
    GccRuntime,
    Vulkan,
    OpenGl,
    Sdl,
    /// X11, XCB and Wayland client libraries.
    WindowSystem,
    Audio,
    Steam,
    Other,
}

impl Family {
    pub fn of(library: &str) -> Self {
        let base = library.split(".so").next().unwrap_or(library);
        match base {
            "libc" | "libm" | "libpthread" | "libdl" | "librt" | "libresolv" | "libutil"
            | "libanl" | "libmvec" | "ld-linux-x86-64" => Self::Glibc,
            "libstdc++" | "libgcc_s" => Self::GccRuntime,
            "libvulkan" => Self::Vulkan,
            "libGL" | "libEGL" | "libGLX" | "libOpenGL" | "libGLESv2" => Self::OpenGl,
            b if b.starts_with("libSDL") => Self::Sdl,
            b if b.starts_with("libX")
                || b.starts_with("libxcb")
                || b.starts_with("libwayland")
                || b.starts_with("libxkbcommon") =>
            {
                Self::WindowSystem
            }
            b if b.starts_with("libasound")
                || b.starts_with("libpulse")
                || b.starts_with("libpipewire")
                || b.starts_with("libopenal") =>
            {
                Self::Audio
            }
            "libsteam_api" | "steamclient" => Self::Steam,
            _ => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Glibc => "glibc",
            Self::GccRuntime => "GCC runtime",
            Self::Vulkan => "Vulkan",
            Self::OpenGl => "OpenGL",
            Self::Sdl => "SDL",
            Self::WindowSystem => "window system",
            Self::Audio => "audio",
            Self::Steam => "Steam",
            Self::Other => "other",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Family;

    #[test]
    fn classifies_common_linux_libraries() {
        let cases = [
            ("libc.so.6", Family::Glibc),
            ("libpthread.so.0", Family::Glibc),
            ("ld-linux-x86-64.so.2", Family::Glibc),
            ("libstdc++.so.6", Family::GccRuntime),
            ("libgcc_s.so.1", Family::GccRuntime),
            ("libvulkan.so.1", Family::Vulkan),
            ("libGL.so.1", Family::OpenGl),
            ("libSDL3.so.0", Family::Sdl),
            ("libSDL2-2.0.so.0", Family::Sdl),
            ("libX11.so.6", Family::WindowSystem),
            ("libwayland-client.so.0", Family::WindowSystem),
            ("libpulse.so.0", Family::Audio),
            ("libsteam_api.so", Family::Steam),
            ("libfreetype.so.6", Family::Other),
        ];
        for (library, family) in cases {
            assert_eq!(Family::of(library), family, "{library}");
        }
    }
}
