//! Build-time finishing for the static bundle, run on Trunk's staging
//! directory by the `post_build` hook (see `Trunk.toml`):
//!
//! - writes [`bcc_web::App`]'s initial HTML into the `<body>` of `index.html`
//!   (static SSR; the browser hydrates it), and
//! - moves the engine Worker's files, which Trunk does not hash, into
//!   `engine-<hash>/` named from their contents, adds the Worker's entry
//!   script, and records its URL in a `<meta>` tag, and
//! - renames the Monaco editor bundle Trunk copied to `editor/` into
//!   `editor-<hash>/` and records its entry module's URL the same way,
//!
//! so every asset except `index.html` has a content-addressed URL a CDN can
//! cache indefinitely. (The web fonts in `fonts/` carry their package version
//! in their names.)
//!
//! The initial state is fixed (the first demo, no engine), so the output is
//! the same on every build. Nothing here serves requests.

use std::{
    env,
    fs,
    io,
    path::{
        Path,
        PathBuf,
    },
    process::ExitCode,
};

use leptos::prelude::*;

fn main() -> ExitCode {
    let Some(staging) = env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: prerender <trunk-staging-dir>");
        return ExitCode::FAILURE;
    };
    match finish(&staging) {
        | Ok(()) => ExitCode::SUCCESS,
        | Err(error) => {
            eprintln!("prerender: {}: {error}", staging.display());
            ExitCode::FAILURE
        },
    }
}

fn finish(staging: &Path) -> Result<(), String> {
    let index = staging.join("index.html");
    let page = fs::read_to_string(&index).map_err(|error| format!("index.html: {error}"))?;
    let files = worker_files(staging).map_err(|error| format!("engine Worker: {error}"))?;
    if files.len() != 2 {
        return Err(format!(
            "expected Trunk's engine-worker.js and engine-worker_bg.wasm, found {files:?}"
        ));
    }
    let hash = hash_files(&files).map_err(|error| format!("engine Worker: {error}"))?;
    let directory = format!("engine-{hash:016x}");
    let editor = hash_editor(staging).map_err(|error| format!("editor bundle: {error}"))?;
    let page = render_into(
        &page,
        &format!("./{directory}/{WORKER_LOADER}"),
        &format!("./{editor}/{EDITOR_ENTRY}"),
    )
    .ok_or("index.html has no empty <body> slot, or was already rendered")?;
    // The loader refers to its siblings by relative URL.
    let target = staging.join(&directory);
    fs::create_dir(&target).map_err(|error| format!("{directory}/: {error}"))?;
    for file in &files {
        let name = file.file_name().unwrap_or_default();
        fs::rename(file, target.join(name))
            .map_err(|error| format!("{}: {error}", file.display()))?;
    }
    fs::write(target.join(WORKER_LOADER), LOADER)
        .map_err(|error| format!("{WORKER_LOADER}: {error}"))?;
    fs::write(&index, page).map_err(|error| format!("index.html: {error}"))
}

/// The editor bundle's entry module, built by `web/editor/build.mjs`.
const EDITOR_ENTRY: &str = "main.js";

/// Rename the copied `editor/` directory after its contents; returns the new
/// name.
fn hash_editor(staging: &Path) -> io::Result<String> {
    let source = staging.join("editor");
    let mut files = Vec::new();
    for entry in fs::read_dir(&source)? {
        let path = entry?.path();
        if path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    if !files.iter().any(|file| file.ends_with(EDITOR_ENTRY)) {
        return Err(io::Error::other(format!(
            "editor/{EDITOR_ENTRY} is missing; run `npm run build` in web/editor"
        )));
    }
    let directory = format!("editor-{:016x}", hash_files(&files)?);
    fs::rename(&source, staging.join(&directory))?;
    Ok(directory)
}

/// The Worker's entry script. A Worker must start from a script, and
/// wasm-bindgen's `no-modules` output only defines an initializer, so this
/// loads the glue and starts the Wasm, whose `main` then runs. Trunk's own
/// `data-loader-shim` uses an initializer form wasm-bindgen deprecates.
const WORKER_LOADER: &str = "engine-worker_loader.js";
const LOADER: &str = "importScripts(\"./engine-worker.js\");\nwasm_bindgen({ module_or_path: \
                      \"./engine-worker_bg.wasm\" });\n";

/// Trunk's output for the `engine-worker` binary: its wasm-bindgen script and
/// Wasm module, in sorted order.
fn worker_files(staging: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(staging)? {
        let path = entry?.path();
        if path.is_file()
            && path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("engine-worker"))
        {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Marks a rendered page so a second run cannot nest two apps.
const RENDERED: &str = "<!-- prerendered by bcc-web -->";

/// `page` with the app's HTML at the start of its body and the engine Worker
/// and editor bundle URLs at the end of its head; `None` if it was already
/// rendered.
fn render_into(page: &str, worker: &str, editor: &str) -> Option<String> {
    const BODY: &str = "<body>";
    const HEAD_END: &str = "</head>";
    if page.contains(RENDERED) {
        return None;
    }
    // The last matches: the head may mention the tags in a comment.
    let head_end = page.rfind(HEAD_END)?;
    let body = page.rfind(BODY)? + BODY.len();
    if body < head_end {
        return None;
    }
    let owner = Owner::new();
    let html = owner.with(|| view! { <bcc_web::App /> }.to_html());
    // Hydration walks the body from its first child, so the app markup must
    // come first; the <noscript> fallback stays after it.
    Some(format!(
            "{}<meta name=\"{}\" content=\"{worker}\">\n    <meta name=\"{}\" \
             content=\"{editor}\">\n  {}{html}{RENDERED}{}",
            &page[..head_end],
            bcc_web::ENGINE_META,
            bcc_web::EDITOR_META,
            &page[head_end..body],
            page[body..].trim_start()
        ))
}

/// A stable 64-bit FNV-1a hash of each file's name and contents, in order. It
/// names cache entries; it is not a security boundary.
fn hash_files(files: &[PathBuf]) -> io::Result<u64> {
    let mut hash = 0xCBF2_9CE4_8422_2325_u64;
    for file in files {
        let name = file.file_name().unwrap_or_default().to_string_lossy();
        for byte in name.bytes().chain([0]).chain(fs::read(file)?).chain([0]) {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01B3);
        }
    }
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_once_at_the_start_of_the_body() {
        let page =
            "<head><!-- the <body> --></head>\n  <body>\n    <noscript>x</noscript>\n  </body>";
        let rendered = render_into(page, "./engine-1/engine-worker.js", "./editor-1/main.js")
            .expect("an empty body renders");
        let body = rendered.rfind("<body>").expect("body kept") + "<body>".len();
        assert!(rendered[body..].starts_with("<a href=\"#workbench\" class=\"skip-link\">"));
        assert!(rendered.contains(&format!("{RENDERED}<noscript>")));
        assert!(rendered.starts_with(
            "<head><!-- the <body> --><meta name=\"bcc-engine-worker\" \
             content=\"./engine-1/engine-worker.js\">\n    <meta name=\"bcc-editor\" \
             content=\"./editor-1/main.js\">"
        ));
        assert_eq!(
            render_into(
                &rendered,
                "./engine-1/engine-worker.js",
                "./editor-1/main.js"
            ),
            None,
            "a rendered page is not rendered again"
        );
    }

    #[test]
    fn moves_the_worker_into_a_content_hashed_directory() {
        let staging = env::temp_dir().join(format!("bcc-web-staging-{}", std::process::id()));
        fs::create_dir_all(&staging).unwrap();
        let page = "<html><head></head>\n<body>\n</body></html>";
        let build = |wasm: &str| {
            fs::write(staging.join("index.html"), page).unwrap();
            fs::write(staging.join("engine-worker.js"), "glue").unwrap();
            fs::write(staging.join("engine-worker_bg.wasm"), wasm).unwrap();
            fs::write(staging.join("bcc-web-1.js"), "app").unwrap();
            fs::create_dir_all(staging.join("editor")).unwrap();
            fs::write(staging.join("editor/main.js"), "editor").unwrap();
            finish(&staging).unwrap();
            assert!(
                !staging.join("editor").exists(),
                "the editor bundle is renamed"
            );
            for entry in fs::read_dir(&staging).unwrap() {
                let path = entry.unwrap().path();
                if path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("editor-")
                {
                    fs::remove_dir_all(path).unwrap();
                }
            }
            let page = fs::read_to_string(staging.join("index.html")).unwrap();
            let start = page.find("content=\"./").unwrap() + "content=\"./".len();
            let url = &page[start..page[start..].find('"').unwrap() + start];
            let directory = url
                .strip_suffix(&format!("/{WORKER_LOADER}"))
                .unwrap()
                .to_owned();
            assert_eq!(
                fs::read_to_string(staging.join(&directory).join(WORKER_LOADER)).unwrap(),
                LOADER
            );
            for name in ["engine-worker.js", "engine-worker_bg.wasm"] {
                assert!(
                    staging.join(&directory).join(name).is_file(),
                    "{name} moved"
                );
                assert!(!staging.join(name).exists(), "{name} left the root");
            }
            assert!(staging.join("bcc-web-1.js").is_file(), "other assets stay");
            fs::remove_dir_all(staging.join(&directory)).unwrap();
            directory
        };
        let first = build("a");
        assert!(first.starts_with("engine-") && first.len() == "engine-".len() + 16);
        assert_eq!(
            build("a"),
            first,
            "the same contents give the same directory"
        );
        assert_ne!(build("b"), first, "changed contents give a new directory");
        fs::remove_dir_all(&staging).unwrap();
    }
}
