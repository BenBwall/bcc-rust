//! The private COFF directive vocabulary; ELF section interpretation stays
//! separate.

use super::{
    super::{
        Item,
        Kind,
        encoder,
        listing::{
            expression,
            symbol_name,
            unquote,
            unsigned,
        },
    },
    Coff,
    Error,
    Result,
    UnwindSections,
};
use crate::util::bump::{
    ArenaMap,
    Bump,
};

pub(super) fn parse<'a>(text: &'a str, gnu: bool, arena: &'a Bump) -> Result<Coff<'a>> {
    let mut coff = Coff::new(arena, gnu);
    for (name, flags) in [
        (".text", 0x6000_0020),
        (".data", 0xC000_0040),
        (".bss", 0xC000_0080),
    ] {
        _ = coff.section(name, flags, None, 0)?;
    }
    let mut section = 0;
    let mut definition = None;
    let mut storage_classes = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, arena);
    for line in text.lines().map(str::trim).filter(|s| !s.is_empty()) {
        // .def/.scl/.type/.endef may share a line in hand-written fixtures.
        // Semicolons inside quoted symbol names and strings remain literal.
        let mut quoted = false;
        let mut escaped = false;
        for statement in line
            .split(|ch| {
                if ch == '"' && !escaped {
                    quoted = !quoted;
                }
                let separator = ch == ';' && !quoted;
                escaped = ch == '\\' && !escaped;
                separator
            })
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let object = &mut coff.object;
            let kind = if let Some(name) = statement.strip_suffix(':') {
                symbol_name(name)?;
                let id = object.symbol(name);
                if object.symbols[id].section.is_some() || object.symbols[id].alias.is_some() {
                    return Err(Error("duplicate COFF symbol definition"));
                }
                object.symbols[id].section = Some(section);
                Kind::Label(id)
            } else if let Some((name, target)) = assignment(statement) {
                symbol_name(name)?;
                symbol_name(target)?;
                let id = object.symbol(name);
                if object.symbols[id].section.is_some() || object.symbols[id].alias.is_some() {
                    return Err(Error("duplicate COFF alias definition"));
                }
                object.symbols[id].alias = Some(object.symbol(target));
                continue;
            } else {
                let (name, args) = statement
                    .split_once(char::is_whitespace)
                    .map_or((statement, ""), |(a, b)| (a, b.trim()));
                let mut parts = split(args);
                let first = parts.next().unwrap_or("");
                match name {
                    | ".text" | ".data" | ".bss" => {
                        arity(args, 0, 0)?;
                        section = match name {
                            | ".text" => 0,
                            | ".data" => 1,
                            | _ => 2,
                        };
                        continue;
                    },
                    | ".section" => {
                        section = parse_section(&mut coff, args)?;
                        continue;
                    },
                    | ".globl" | ".weak" => {
                        arity(args, 1, 1)?;
                        symbol_name(args)?;
                        let id = object.symbol(args);
                        if object.symbols[id].name.starts_with(".L") {
                            return Err(Error("unsupported COFF temporary symbol attributes"));
                        }
                        if name == ".weak" && storage_classes.get(&id) == Some(&3) {
                            return Err(Error("unsupported static COFF weak symbol"));
                        }
                        if name == ".globl" && object.symbols[id].binding == 2 {
                            return Err(Error("global binding after weak binding"));
                        }
                        object.symbols[id].binding = if name == ".weak" { 2 } else { 1 };
                        continue;
                    },
                    | ".def" => {
                        arity(args, 1, 1)?;
                        symbol_name(args)?;
                        if definition.is_some() {
                            return Err(Error("nested COFF symbol definition"));
                        }
                        definition = Some(object.symbol(args));
                        continue;
                    },
                    | ".scl" | ".type" => {
                        arity(args, 1, 1)?;
                        let id = definition.ok_or(Error("COFF symbol attribute outside .def"))?;
                        match (name, unsigned(args)?) {
                            // MC keeps weakness independent of storage class.
                            // Static weak fallbacks need separate class metadata;
                            // refuse them in either directive order for now.
                            | (".scl", 3) if object.symbols[id].binding == 2 =>
                                return Err(Error("unsupported static COFF weak symbol")),
                            | (".scl", class @ (2 | 3)) => {
                                if object.symbols[id].binding != 2 {
                                    object.symbols[id].binding = u8::from(class == 2);
                                }
                                _ = storage_classes.insert(id, class);
                            },
                            | (".type", 32) => object.symbols[id].kind = 2,
                            | (".type", 0) => object.symbols[id].kind = 0,
                            | _ => return Err(Error("unsupported COFF symbol attribute")),
                        }
                        continue;
                    },
                    | ".endef" => {
                        arity(args, 0, 0)?;
                        if definition.take().is_none() {
                            return Err(Error("COFF .endef without .def"));
                        }
                        continue;
                    },
                    | ".p2align" => {
                        arity(args, 1, 2)?;
                        let power = u32::try_from(unsigned(first)?)
                            .map_err(|_| Error("COFF alignment too large"))?;
                        if power > 13 {
                            return Err(Error("COFF alignment exceeds 8192"));
                        }
                        let align = 1_u64 << power;
                        object.sections[section].align = object.sections[section].align.max(align);
                        let fill = parts
                            .next()
                            .map(|n| {
                                u8::try_from(unsigned(n)?)
                                    .map_err(|_| Error("COFF fill exceeds byte"))
                            })
                            .transpose()?;
                        Kind::Align {
                            align,
                            fill,
                            max: align,
                        }
                    },
                    | ".zero" => {
                        arity(args, 1, 1)?;
                        Kind::Zero(unsigned(args)?)
                    },
                    | ".byte" | ".short" | ".long" | ".quad" | ".rva" | ".secrel32" | ".secidx" => {
                        arity(args, 1, usize::MAX)?;
                        let width = match name {
                            | ".byte" => 1,
                            | ".short" | ".secidx" => 2,
                            | ".quad" => 8,
                            | _ => 4,
                        };
                        let reloc = match name {
                            | ".rva" => 0x0001_0003,
                            | ".secrel32" => 0x0001_000B,
                            | ".secidx" => 0x0001_000A,
                            | _ => 0,
                        };
                        for value in split(args) {
                            _ = expression(value)?;
                            object.items.push(Item {
                                section,
                                offset: 0,
                                kind: if reloc == 0 {
                                    Kind::Value(width, value)
                                } else {
                                    Kind::CoffValue(width, value, reloc)
                                },
                            });
                        }
                        continue;
                    },
                    | ".ascii" => {
                        arity(args, 1, 1)?;
                        let bytes = ascii(args, arena)?;
                        Kind::Bytes(bytes)
                    },
                    | ".seh_proc" => {
                        arity(args, 1, 1)?;
                        symbol_name(args)?;
                        if !coff.unwind.iter().any(|u| u.text == section) {
                            // MinGW streams xdata, including discardable
                            // subsections. MSVC defers associative COMDAT
                            // unwind sections to EOF.
                            let xdata = if gnu || coff.object.sections[section].group.is_none() {
                                Some(coff.unwind_section(section, ".xdata")?)
                            } else {
                                None
                            };
                            coff.unwind.push(UnwindSections {
                                text: section,
                                xdata,
                                pdata: None,
                            });
                        }
                        Kind::Cfi(name, args)
                    },
                    | name if name.starts_with(".seh_") => {
                        match name {
                            | ".seh_endproc" | ".seh_endprologue" => arity(args, 0, 0)?,
                            | ".seh_pushreg" | ".seh_stackalloc" => arity(args, 1, 1)?,
                            | ".seh_savexmm" | ".seh_setframe" | ".seh_savereg" =>
                                arity(args, 2, 2)?,
                            | _ => return Err(Error("unknown SEH directive")),
                        }
                        Kind::Cfi(name, args)
                    },
                    | name if name.starts_with('.') =>
                        return Err(Error("unknown COFF object directive")),
                    | "jmp" => {
                        symbol_name(args)?;
                        Kind::Branch {
                            condition: None,
                            target:    object.symbol(args),
                            near:      false,
                        }
                    },
                    | name if name.starts_with('j') && name != "jmpq" => {
                        symbol_name(args)?;
                        Kind::Branch {
                            condition: Some(encoder::condition(&name[1..])?),
                            target:    object.symbol(args),
                            near:      false,
                        }
                    },
                    | _ => {
                        let code = encoder::instruction(name, args)?;
                        if code.fixup.is_some_and(|f| {
                            matches!(f.kind, 9 | 22 | 23 | 41 | 42)
                                || f.expression.ends_with("@PLT")
                        }) {
                            return Err(Error("unsupported ELF relocation in COFF listing"));
                        }
                        Kind::Code(code)
                    },
                }
            };
            coff.object.items.push(Item {
                section,
                offset: 0,
                kind,
            });
        }
    }
    if definition.is_some() {
        return Err(Error("unterminated COFF symbol definition"));
    }
    Ok(coff)
}

fn parse_section<'a>(coff: &mut Coff<'a>, args: &'a str) -> Result<usize> {
    arity(args, 1, 4)?;
    let mut parts = split(args);
    let name = unquote(parts.next().ok_or(Error("missing COFF section"))?);
    if name.is_empty()
        || name
            .bytes()
            .any(|b| !b.is_ascii_alphanumeric() && !matches!(b, b'_' | b'.' | b'$' | b'-'))
    {
        return Err(Error("unsupported COFF section name"));
    }
    if matches!(name, ".xdata" | ".pdata")
        || name.starts_with(".xdata$")
        || name.starts_with(".pdata$")
    {
        return Err(Error(
            "user section conflicts with generated COFF unwind section",
        ));
    }
    let flags = parts
        .next()
        .map(|text| {
            text.strip_prefix('"')
                .and_then(|text| text.strip_suffix('"'))
                .ok_or(Error("COFF section flags must be quoted"))
        })
        .transpose()?;
    let mut characteristics = flags
        .map(|flags| super::flags::parse(name, flags))
        .transpose()?
        .unwrap_or(0xC000_0040);
    let mut group = None;
    let mut selection = 0;
    if let Some(kind) = parts.next() {
        selection = match kind {
            | "discard" => 2,
            | "one_only" => 1,
            | "same_size" => 3,
            | "same_contents" => 4,
            | "largest" => 6,
            | "newest" => 7,
            | _ => return Err(Error("unknown COFF COMDAT selection")),
        };
        let signature = parts.next().ok_or(Error("missing COFF COMDAT signature"))?;
        symbol_name(signature)?;
        group = Some(coff.object.symbol(signature));
        characteristics |= 0x1000;
    }
    if flags.is_none()
        && group.is_none()
        && let Some(id) = coff
            .object
            .sections
            .iter()
            .position(|s| s.name == name && s.group.is_none())
    {
        return Ok(id);
    }
    // MCContext pre-creates these sections; its interning keeps their
    // attributes even when a later directive requests different flags.
    if group.is_none() {
        characteristics = super::flags::predefined(name).unwrap_or(characteristics);
    }
    coff.section(name, characteristics, group, selection)
}

fn arity(args: &str, min: usize, max: usize) -> Result<()> {
    let valid = if args.is_empty() {
        min == 0
    } else {
        (min..=max).contains(&split(args).count()) && split(args).all(|s| !s.is_empty())
    };
    if valid {
        Ok(())
    } else {
        Err(Error("unsupported COFF directive operands"))
    }
}

fn assignment(line: &str) -> Option<(&str, &str)> {
    let mut quoted = false;
    let mut escaped = false;
    for (at, ch) in line.char_indices() {
        if ch == '"' && !escaped {
            quoted = !quoted;
        }
        if ch == '=' && !quoted {
            return Some((line[..at].trim(), line[at + 1..].trim()));
        }
        escaped = ch == '\\' && !escaped;
    }
    None
}

fn ascii<'a>(text: &str, arena: &'a Bump) -> Result<&'a str> {
    use crate::util::bump::ArenaString;
    let text = text
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or(Error("invalid COFF string"))?;
    let mut out = ArenaString::new_in(arena);
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        out.push(if ch == '\\' {
            match chars.next() {
                | Some('"') => '"',
                | Some('\\') => '\\',
                | _ => return Err(Error("unsupported COFF string escape")),
            }
        } else if ch == '"' || !ch.is_ascii() {
            return Err(Error("unsupported COFF string character"));
        } else {
            ch
        });
    }
    Ok(out.into_str())
}

// COFF export strings contain escaped quotes around punctuation-bearing names.
fn split(text: &str) -> impl Iterator<Item = &str> {
    let mut quoted = false;
    let mut escaped = false;
    let mut depth = 0_u32;
    text.split(move |ch| {
        match ch {
            | '"' if !escaped => quoted = !quoted,
            | '(' if !quoted => depth += 1,
            | ')' if !quoted => depth = depth.saturating_sub(1),
            | _ => {},
        }
        escaped = ch == '\\' && !escaped;
        ch == ',' && !quoted && depth == 0
    })
    .map(str::trim)
}

#[cfg(test)]
mod tests {
    use crate::{
        backend::native::tests::object_coff::equivalent,
        target::Target,
    };

    #[test]
    fn coff_storage_class_preserves_weak_binding() {
        for text in [
            ".weak f\n.def f;.scl 2;.type 32;.endef\nf:\nretq\n.globl g\ng:\nretq\n",
            ".def f;.scl 2;.type 32;.endef\n.weak f\nf:\nretq\n.globl g\ng:\nretq\n",
            ".weak f\n.def f;.scl 2;.type 32;.endef\n.quad f\n",
        ] {
            for target in [Target::WindowsGnu, Target::WindowsMsvc] {
                equivalent(text, target).unwrap();
            }
        }
        for text in [
            ".weak f\n.def f;.scl 3;.type 32;.endef\nf:\nretq\n",
            ".def f;.scl 3;.type 32;.endef\n.weak f\nf:\nretq\n",
        ] {
            let arena = crate::util::bump::Bump::new();
            assert!(super::parse(text, false, &arena).is_err());
        }
    }
}
