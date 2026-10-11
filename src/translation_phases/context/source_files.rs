//! Loads and interns source files, configures header searches, and keeps
//! original text and lazy line indices for diagnostics. Presumed filenames
//! refer back to the physical source bytes.
//!
//! C99: phases 1-7, §5.1.1.2 paragraph 1, pp. 9-10; PDF pp. 21-22.
//! Diagnostic locations: §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
//! Directive interpretation remains in preprocessing.

use std::{
    cell::OnceCell,
    ffi::OsStr,
    path::Path,
};

use rustc_hash::FxBuildHasher;

use super::{
    Context,
    source_text::SourceText,
};
use crate::{
    headers::HeaderSearch,
    translation_phases::{
        ErrorSeverity,
        provenance::{
            SourceVector,
            SourceVectors,
        },
    },
    util::bump::{
        ArenaSet,
        ArenaString,
        ArenaVec,
        Bump,
    },
};

impl<'tu> Context<'tu> {
    /// Reads and retains an included file without a temporary heap string.
    pub(crate) fn read_source_file(&mut self, index: u32) -> std::io::Result<&'tu str> {
        if let Ok(name) = self
            .get_source_file(index)
            .strip_prefix(crate::headers::DIRECTORY)
            && let Some(text) = crate::headers::text(name)
        {
            let text = if let Some((before, after)) = text.split_once("@MB_LEN_MAX@") {
                use std::fmt::Write as _;
                let mut rendered = ArenaString::new_in(self.tu);
                write!(
                    rendered,
                    "{before}{}{after}",
                    self.configuration.target().layout().mb_len_max
                )
                .unwrap();
                rendered.into_str()
            } else {
                text
            };
            self.record_arena_source_text(index, text);
            return Ok(text);
        }
        let text = self.tu.read_to_str_lossy(self.get_source_file(index))?;
        self.record_arena_source_text(index, text);
        Ok(text)
    }

    pub(crate) fn intern_source_file(&mut self, path: &Path) -> u32 {
        let tu = self.tu;
        self.source_files
            .intern_by(path, || Self::alloc_path(tu, path))
    }

    pub(crate) fn record_arena_source_text(&mut self, index: u32, text: &'tu str) {
        let index = index as usize;
        if self.source_texts.len() <= index {
            self.source_texts.resize_with(index + 1, || None);
        }
        self.source_texts[index] = Some(SourceText {
            text,
            line_starts: OnceCell::new(),
        });
    }

    /// Records where headers are searched for, in order. As in GCC and
    /// Clang, an `-I` entry that duplicates a system entry is dropped so the
    /// directory stays a system directory, and each group keeps only the
    /// first of its own duplicates; path equality ignores redundant `.`
    /// components. The `-iquote` group is a separate chain that `<…>`
    /// lookup skips, so, as in Clang, its entries stay user entries. C99:
    /// implementation-defined places, §6.10.2p2-3, pp. 149-150; PDF pp.
    /// 161-162.
    pub(crate) fn set_header_search(&mut self, search: HeaderSearch<'_>) {
        let resource = search
            .resource
            .then_some(Path::new(crate::headers::DIRECTORY));
        let tu = self.tu;
        let system = search
            .system
            .iter()
            .copied()
            .chain(resource)
            .chain(search.after.iter().copied());
        let mut system_paths = ArenaSet::with_hasher_in(FxBuildHasher, tu);
        system_paths.extend(system.clone());
        let mut seen = ArenaSet::with_hasher_in(FxBuildHasher, tu);
        let mut directories = ArenaVec::new_in(tu);
        for (index, group) in [search.quote, search.angled].into_iter().enumerate() {
            let quote = index == 0;
            seen.clear();
            for &path in group {
                if (quote || !system_paths.contains(path)) && seen.insert(path) {
                    directories.push(Self::alloc_path(tu, path));
                }
            }
            if quote {
                self.quote_include_count = directories.len();
            }
        }
        self.system_include_start = directories.len();
        for path in system {
            if seen.insert(path) {
                directories.push(Self::alloc_path(tu, path));
            }
        }
        self.include_directories = directories.leak();
    }

    /// Gives a presumed filename its own identity without interning it as an
    /// opened file. Its system-header classification follows `physical`;
    /// its rendered name remains `path`.
    /// C99: §6.10.4 paragraph 4, p. 158; PDF p. 170.
    pub(crate) fn add_presumed_source_file(&mut self, path: &Path, physical: u32) -> u32 {
        let index = self
            .source_files
            .push_unindexed(Self::alloc_path(self.tu, path));
        _ = self.presumed_files.insert(index, physical);
        index
    }

    /// Stable virtual paths do not depend on the host's path separator.
    /// C99: implementation-defined headers §6.10.2p2-3, pp. 149-150;
    /// PDF pp. 161-162.
    pub(crate) fn intern_builtin_header(&mut self, name: &Path) -> u32 {
        use std::fmt::Write as _;
        let mut path = ArenaString::new_in(self.tu);
        write!(path, "{}/{}", crate::headers::DIRECTORY, name.display()).unwrap();
        self.intern_source_file(Path::new(&*path))
    }

    /// Registers synthetic source text under a fresh identity, even when
    /// `path` names an earlier input, so diagnostics retained from each
    /// input keep quoting their own text.
    pub(crate) fn add_synthetic_source_file(&mut self, path: &Path, text: &'tu str) -> u32 {
        let index = self
            .source_files
            .push_unindexed(Self::alloc_path(self.tu, path));
        self.record_arena_source_text(index, text);
        index
    }

    /// Makes `file` a system header from byte `from` on. GCC extension
    /// over the implementation-defined header places of C99 §6.10.2p2-3,
    /// pp. 149-150; PDF pp. 161-162.
    pub(crate) fn mark_system_header(&mut self, file: u32, from: u32) {
        let start = self.system_headers.entry(file).or_insert(from);
        *start = (*start).min(from);
    }

    /// Whether any part of `file` is a system header.
    pub(crate) fn has_system_header_part(&self, file: u32) -> bool {
        self.system_headers.contains_key(&file)
    }

    /// Whether `vector` starts in a physical system header, independently
    /// of its presumed filename.
    /// C99: presumed filenames, §6.10.4 paragraph 4, p. 158; PDF p. 170.
    pub(crate) fn in_system_header(&self, vector: &SourceVector) -> bool {
        let physical = self
            .presumed_files
            .get(&vector.source_file_index)
            .copied()
            .unwrap_or(vector.source_file_index);
        self.system_headers
            .get(&physical)
            .is_some_and(|&from| vector.index >= from)
    }

    /// Whether a diagnostic is withheld because it arises in a system
    /// header, as GCC and Clang withhold them: a warning, or an extension
    /// diagnostic at any severity, whose location is spelled in a system
    /// header. The location's first source vector is its spelling: for a
    /// token from a macro expansion, the macro's replacement list. Errors
    /// are never withheld.
    /// The primary range must be non-empty even if the diagnostic is withheld.
    /// C99: §5.1.1.3 paragraph 1 and footnote 8, p. 11; PDF p. 23.
    pub(crate) fn withholds(
        &self,
        severity: ErrorSeverity,
        extension: bool,
        source_vectors: SourceVectors,
    ) -> bool {
        #[cfg(debug_assertions)]
        let _ = self.diagnostic_position(source_vectors);
        (extension || severity == ErrorSeverity::Warning)
            && !self.system_headers.is_empty()
            && source_vectors.length() != 0
            && self.in_system_header(self.first_source_vector(source_vectors))
    }

    /// The configured search entries a lookup visits, with their indices:
    /// from `start` when given (`#include_next`), otherwise every entry for
    /// a `"…"` name and every entry after the `-iquote` ones for a `<…>`
    /// name. C99: implementation-defined search, §6.10.2p2-3, pp. 149-150;
    /// PDF pp. 161-162.
    pub(crate) fn configured_include_directories(
        &self,
        is_system_header: bool,
        start: Option<usize>,
    ) -> impl Iterator<Item = (usize, &'tu Path)> + Clone + use<'tu> {
        let directories: &'tu [&'tu Path] = self.include_directories;
        let start = start
            .unwrap_or(if is_system_header {
                self.quote_include_count
            } else {
                0
            })
            .min(directories.len());
        directories[start..]
            .iter()
            .copied()
            .enumerate()
            .map(move |(offset, directory)| (start + offset, directory))
    }

    pub(super) fn alloc_path(tu: &'tu Bump, path: &Path) -> &'tu Path {
        let bytes = tu.alloc_slice_copy(path.as_os_str().as_encoded_bytes());
        // SAFETY: These are the complete encoded bytes of an OsStr from this
        // process and target, copied without splitting or changing them.
        Path::new(unsafe { OsStr::from_encoded_bytes_unchecked(bytes) })
    }

    /// Remembers the text a source file was translated from.
    pub(crate) fn record_source_text(&mut self, index: u32, text: &str) {
        let text = self.tu.alloc_str(text);
        self.record_arena_source_text(index, text);
    }

    /// Returns the text of a source file, if it was recorded.
    pub(crate) fn source_text(&self, index: u32) -> Option<&str> {
        self.source_texts
            .get(index as usize)?
            .as_ref()
            .map(|source| source.text)
    }

    /// Byte offsets at which physical lines start, built only when a
    /// diagnostic needs them and retained across renderers. LF, CRLF, and
    /// lone CR each end a line, just as in initial processing.
    pub(crate) fn source_line_starts(&self, index: u32) -> Option<&'tu [usize]> {
        let source = self.source_texts.get(index as usize)?.as_ref()?;
        Some(*source.line_starts.get_or_init(|| {
            let bytes = source.text.as_bytes();
            self.tu.alloc_slice_fill_iter(
                std::iter::once(0).chain(
                    bytes
                        .iter()
                        .enumerate()
                        .filter(|&(index, &byte)| {
                            byte == b'\n' || (byte == b'\r' && bytes.get(index + 1) != Some(&b'\n'))
                        })
                        .map(|(index, _)| index + 1),
                ),
            )
        }))
    }

    /// Returns the exact source spelling covered by a single-segment range.
    pub(crate) fn source_spelling(&self, source_vectors: SourceVectors) -> Option<&str> {
        let [vector] = self.get_source_vectors(source_vectors) else {
            return None;
        };
        self.source_text(vector.source_file_index)?
            .get(vector.range())
    }

    pub(crate) fn get_source_file(&self, index: u32) -> &'tu Path {
        self.source_files[index]
    }

    /// Whether the configured search entry at `index` is a system directory.
    pub(crate) fn is_system_include_directory(&self, index: usize) -> bool {
        index >= self.system_include_start
    }
}
