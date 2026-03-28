use std::path::PathBuf;

fn main() {
    let libde265_dir: PathBuf = ["../../../vendor/libde265/libde265"].iter().collect();

    let sources: Vec<PathBuf> = std::fs::read_dir(&libde265_dir)
        .expect("can't read libde265 dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|ext| ext == "cc")
                && !p
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .contains("en265")
                && !p
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .contains("visualize")
                && !p
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .contains("image-io")
        })
        .collect();

    // Use emscripten's em++ as the C++ compiler — it provides the full sysroot
    let emcc_path = "/usr/lib/emscripten";
    let compiler = format!("{}/em++", emcc_path);

    // cc crate adds --target=wasm32-unknown-unknown which em++ rejects.
    // Use TARGET override to make em++ use its default wasm32-emscripten target.
    // The .o files are compatible since both are wasm32.
    std::env::set_var("TARGET", "wasm32-unknown-emscripten");

    cc::Build::new()
        .compiler(&compiler)
        .cpp(true)
        .std("c++17")
        .opt_level(3)
        .include("../../../vendor/libde265")
        .define("HAVE_STDINT_H", None)
        .flag("-fno-exceptions")
        .flag("-nostdlib")
        .cpp_set_stdlib(None)
        .files(&sources)
        .warnings(false)
        .compile("de265");

    // Don't let cc emit cargo:rustc-link-lib=stdc++ — we handle it ourselves
    println!("cargo:rustc-link-lib=static=c++");
    println!("cargo:rustc-link-lib=static=c++abi");

    // Point to emscripten's pre-built wasm libraries
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/charles".to_string());
    let em_lib = format!("{}/.cache/emscripten/sysroot/lib/wasm32-emscripten", home);
    println!("cargo:rustc-link-search=native={}", em_lib);
}
