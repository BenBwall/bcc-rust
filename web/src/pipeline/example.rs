//! One concrete example of what a stage did to the program on screen, read
//! from bcc's own output for it: the "In this program" line under each
//! stage's hand-off. The whole-path lesson has hand-written notes instead
//! (`demos::Demo::path`); every other program gets these.

use super::{
    Census,
    Compilation,
    Stage,
    token_line,
};

/// A sentence on what `stage` did to `source`, compiled as `compilation`,
/// or `None` when the stage has no output to describe.
pub fn example(stage: Stage, compilation: &Compilation, source: &str) -> Option<String> {
    match stage {
        | Stage::Lines => lines(compilation),
        | Stage::PpTokens => pp_tokens(compilation, source),
        | Stage::Preprocessor => preprocessor(compilation),
        | Stage::Tokens => tokens(compilation),
        | Stage::SyntaxTree => syntax_tree(compilation),
        | Stage::SemanticTypes => semantic_types(compilation),
        | Stage::Ir | Stage::Llvm => {
            let census = Census::of(stage, compilation.output(stage)?);
            let what = if stage == Stage::Ir {
                "bcc IR"
            } else {
                "LLVM IR"
            };
            Some(format!(
                "Here the {what} is {} in {}, with {} and {}.",
                counted(census.instructions, "instruction"),
                counted(census.blocks, "basic block"),
                counted(census.slots, "stack slot"),
                counted_as(census.memory, "load or store", "loads or stores"),
            ))
        },
        | Stage::IrAbi => ir_abi(compilation),
        | Stage::Asm => {
            let assembly = compilation.output(stage)?;
            let census = Census::of(stage, assembly);
            // `callq	printf@PLT`: the callee, reached through the PLT when
            // the C library holds it. `callq	*%r13` calls the address a
            // register holds: a call through a pointer to a function.
            let mut callees: Vec<String> = Vec::new();
            let mut indirect = 0;
            for line in assembly.lines() {
                let mut fields = line.split_whitespace();
                if fields
                    .next()
                    .is_some_and(|mnemonic| mnemonic.starts_with("call"))
                    && let Some(callee) = fields.next()
                {
                    if callee.starts_with('*') {
                        indirect += 1;
                        continue;
                    }
                    let callee = callee.trim_end_matches("@PLT").to_owned();
                    if !callees.contains(&callee) {
                        callees.push(callee);
                    }
                }
            }
            let through = counted_as(
                indirect,
                "call through a function pointer",
                "calls through a function pointer",
            );
            Some(format!(
                "Here bcc selected {}{}.",
                counted(census.instructions, "x86-64 instruction"),
                match (callees.is_empty(), indirect) {
                    | (true, 0) => String::from(", and no function calls another"),
                    | (true, _) => format!(", with {through}"),
                    | (false, 0) => format!(", with calls to {}", names(&callees)),
                    | (false, _) => format!(", with calls to {} and {through}", names(&callees)),
                }
            ))
        },
        | Stage::MachineCode => {
            let census = Census::of(stage, compilation.output(stage)?);
            (census.instructions > 0).then(|| {
                // The average to one decimal place, rounded.
                let tenths =
                    (compilation.code_bytes * 10 + census.instructions / 2) / census.instructions;
                format!(
                    "Here {} encode to {} of code, {}.{} bytes each on average.",
                    counted(census.instructions, "instruction"),
                    counted(compilation.code_bytes, "byte"),
                    tenths / 10,
                    tenths % 10
                )
            })
        },
    }
}

/// Phases 1-2: the first splice, or that there is none.
fn lines(compilation: &Compilation) -> Option<String> {
    compilation.output(Stage::Lines)?;
    let physical = compilation
        .logical_lines
        .last()
        .map_or(0, |line| line.source.1);
    let spliced: Vec<_> = compilation
        .logical_lines
        .iter()
        .filter(|line| line.source.1 > line.source.0)
        .collect();
    Some(match spliced.first() {
        | None => format!(
            "No line ends in a backslash, so each of the {} stays a logical line of its own.",
            counted(physical, "physical line")
        ),
        | Some(first) => format!(
            "{} with a backslash, so phase 2 joins lines {}–{} into one logical line; the {} \
             physical lines make {}.",
            if first.source.1 - first.source.0 == 1 {
                format!("Line {} ends", first.source.0)
            } else {
                format!("Lines {} to {} end", first.source.0, first.source.1 - 1)
            },
            first.source.0,
            first.source.1,
            physical,
            counted(compilation.logical_lines.len(), "logical line")
        ),
    })
}

/// Phase 3: the preprocessing tokens of one short statement, and that its
/// keyword is still an identifier.
fn pp_tokens(compilation: &Compilation, source: &str) -> Option<String> {
    let listing = compilation.output(Stage::PpTokens)?;
    let source_lines: Vec<&str> = source.lines().collect();
    // Each listing line is `line:column kind spelling …`, one per entry,
    // with its exact range in `pp_ranges`.
    let mut entries: Vec<(usize, &str, String)> = Vec::new();
    for (row, range) in listing.lines().zip(&compilation.pp_ranges) {
        let mut fields = row.split_whitespace();
        let (Some(_), Some(kind)) = (fields.next(), fields.next()) else {
            continue;
        };
        if matches!(kind, "whitespace" | "newline") || range.end_line != range.line {
            continue;
        }
        let text = source_lines.get(range.line.checked_sub(1)?)?;
        let Some(spelling) = utf16_slice(text, range.column, range.end_column) else {
            continue;
        };
        entries.push((range.line, kind, spelling.to_owned()));
    }
    // The first line that is a statement or declaration of a few tokens.
    let line = entries.iter().map(|(line, ..)| *line).find(|&line| {
        let on: Vec<_> = entries.iter().filter(|entry| entry.0 == line).collect();
        (3..=10).contains(&on.len())
            && source_lines
                .get(line - 1)
                .is_some_and(|text| !text.trim_end().ends_with('\\'))
            && on.first().is_some_and(|entry| entry.1 == "identifier")
            && on.last().is_some_and(|entry| entry.2 == ";")
    })?;
    let on: Vec<&String> = entries
        .iter()
        .filter(|entry| entry.0 == line)
        .map(|entry| &entry.2)
        .collect();
    let spelled: Vec<String> = on.iter().map(|spelling| format!("`{spelling}`")).collect();
    let keyword = on
        .iter()
        .find(|spelling| KEYWORDS.contains(&spelling.as_str()))
        .map_or_default(|keyword| format!("; `{keyword}` is still just an identifier"));
    Some(format!(
        "Line {line}, `{}`, is cut into {}: {}{keyword}.",
        source_lines.get(line - 1)?.trim(),
        counted(on.len(), "preprocessing token"),
        list(&spelled),
    ))
}

/// Phase 4: what the program's directives and macros did.
fn preprocessor(compilation: &Compilation) -> Option<String> {
    compilation.output(Stage::Preprocessor)?;
    let work = &compilation.preprocessing;
    let mut done = Vec::new();
    for include in &work.includes {
        done.push(if include.found {
            format!(
                "#include pasted in {} ({}).",
                include.spelled(),
                counted(include.lines, "line")
            )
        } else {
            format!("#include found no {}.", include.spelled())
        });
    }
    let defined: Vec<String> = work
        .definitions
        .iter()
        .filter(|definition| definition.kind != "undef")
        .map(|definition| format!("`{}`", definition.name))
        .collect();
    if !defined.is_empty() {
        done.push(format!(
            "#define made {} {}.",
            if defined.len() == 1 {
                "the macro"
            } else {
                "the macros"
            },
            list(&defined)
        ));
    }
    if let Some(first) = work
        .expansions
        .iter()
        .find(|expansion| !expansion.directive)
    {
        let count = work.expansions.len() + work.omitted_expansions;
        done.push(format!(
            "Phase 4 replaced {}, `{}` becoming `{}`.",
            counted_as(count, "macro use", "macro uses"),
            first.invocation,
            first.result
        ));
    }
    let skipped: usize = work
        .skipped
        .iter()
        .map(|(first, last)| last + 1 - first)
        .sum();
    if skipped > 0 {
        done.push(format!("#if skipped {}.", counted(skipped, "line")));
    }
    Some(if done.is_empty() {
        String::from(
            "This program has no directives and no macros, so phase 4 passes its tokens on \
             unchanged.",
        )
    } else {
        done.join(" ")
    })
}

/// Phases 5-7: what a keyword and a constant became, on the first line that
/// has both.
fn tokens(compilation: &Compilation) -> Option<String> {
    let listing = compilation.output(Stage::Tokens)?;
    // `<input>:7:16: integer constant `3` = 3 (int)`
    let own: Vec<(usize, &str)> = listing
        .lines()
        .filter_map(|line| Some((token_line(line)?, line.splitn(4, ':').nth(3)?.trim())))
        .collect();
    let constant = own.iter().find(|(_, token)| {
        token.contains("constant `") && token.ends_with(')') && !token.starts_with("character")
    });
    let keyword = own.iter().find(|(_, token)| token.starts_with("keyword `"));
    let total = own.len();
    let mut parts = Vec::new();
    if let Some((line, token)) = keyword {
        parts.push(format!(
            "`{}` on line {line} is now a keyword",
            quoted(token)?
        ));
    }
    if let Some((line, token)) = constant {
        let ty = token.rsplit_once('(')?.1.trim_end_matches(')');
        parts.push(format!(
            "`{}` on line {line} an {} of type {ty}",
            quoted(token)?,
            token.split_once(" `")?.0
        ));
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!(
        "{}: this file makes {} in all.",
        sentence_case(&parts.join(", and ")),
        counted(total, "token")
    ))
}

/// The parser: the first binary operator and its two operands.
fn syntax_tree(compilation: &Compilation) -> Option<String> {
    let tree = compilation.output(Stage::SyntaxTree)?;
    let rows: Vec<&str> = tree.lines().collect();
    for (index, row) in rows.iter().enumerate() {
        let Some((_, rest)) = row.split_once("binary ") else {
            continue;
        };
        let Some((operator, place)) = rest.split_once(" @") else {
            continue;
        };
        // A header's node is placed by its file's path; the program's own by
        // a line number alone.
        let Some(line) = place
            .split(':')
            .next()
            .and_then(|line| line.parse::<usize>().ok())
        else {
            continue;
        };
        // The operands are the node's children, indented one level more.
        let depth = row.len() - row.trim_start().len() + 2;
        let operand = |side: &str| {
            rows.get(index + 1..)?
                .iter()
                .take_while(|next| next.len() - next.trim_start().len() >= depth)
                .filter(|next| next.len() - next.trim_start().len() == depth)
                .find_map(|next| {
                    let node = next.trim_start().strip_prefix(&format!("{side}: "))?;
                    Some(describe(
                        node.rsplit_once(" @").map_or(node, |(node, _)| node),
                    ))
                })
        };
        let (Some(lhs), Some(rhs)) = (operand("lhs"), operand("rhs")) else {
            continue;
        };
        return Some(format!(
            "On line {line}, `{operator}` is a `binary {operator}` node with its left operand, \
             {lhs}, and its right operand, {rhs}, as its children."
        ));
    }
    let names: Vec<String> = compilation
        .functions
        .iter()
        .map(|function| format!("`{}`", function.name))
        .collect();
    (!names.is_empty()).then(|| {
        format!(
            "The tree's top level holds {}: {}.",
            counted(names.len(), "function definition"),
            list(&names)
        )
    })
}

/// A syntax-tree node in a few words: `identifier n` is `n`.
fn describe(node: &str) -> String {
    if let Some(name) = node.strip_prefix("identifier ") {
        format!("`{name}`")
    } else if let Some(constant) = node.strip_prefix("constant ") {
        format!(
            "the constant `{}`",
            constant.split(' ').next().unwrap_or(constant)
        )
    } else if node.starts_with("binary ") || node.starts_with("unary ") {
        format!("a `{node}` node")
    } else {
        format!("a `{}` node", node.split(' ').next().unwrap_or(node))
    }
}

/// Semantic analysis: a function's type and how many reads it inserted.
fn semantic_types(compilation: &Compilation) -> Option<String> {
    let listing = compilation.output(Stage::SemanticTypes)?;
    let function = compilation.functions.first()?;
    let prefix = format!("function {} type=\"", function.name);
    let ty = listing
        .lines()
        .find_map(|line| line.trim_start().strip_prefix(&prefix)?.split_once('"'))
        .map(|(ty, _)| ty)?;
    let reads = listing
        .lines()
        .filter(|line| line.starts_with("convert ") && line.contains("kind=lvalue"))
        .count();
    Some(format!(
        "`{}` gets the type `{ty}`, and {} of an object's value {} listed as convert lines of \
         kind lvalue.",
        function.name,
        counted(reads, "read"),
        if reads == 1 { "is" } else { "are" }
    ))
}

/// ABI lowering: whether it changed anything for this program.
fn ir_abi(compilation: &Compilation) -> Option<String> {
    let lowered = compilation.output(Stage::IrAbi)?;
    let ir = compilation.output(Stage::Ir)?;
    let before: Vec<&str> = ir.lines().collect();
    // The `abi lowered` header only marks the stage.
    let changed = lowered
        .lines()
        .filter(|line| line.trim() != "abi lowered" && !before.contains(line))
        .count();
    Some(if changed == 0 {
        String::from(
            "Nothing here needs rewriting: every argument and result fits a register, so the \
             ABI-lowered IR is the same as the IR.",
        )
    } else {
        format!(
            "ABI lowering wrote {} that the IR did not have, to pass this program's arguments and \
             results the x86-64 way.",
            counted(changed, "line")
        )
    })
}

/// The spelling between the first pair of backticks of a token line.
fn quoted(token: &str) -> Option<&str> {
    token.split('`').nth(1)
}

/// The part of `line` between two UTF-16 columns (1-based, the end just
/// past the text).
fn utf16_slice(line: &str, start: usize, end: usize) -> Option<&str> {
    let mut column = 1;
    let (mut from, mut to) = (None, None);
    for (index, character) in line.char_indices() {
        if column == start {
            from = Some(index);
        }
        if column == end {
            to = Some(index);
        }
        column += character.len_utf16();
    }
    if column == end {
        to = Some(line.len());
    }
    line.get(from?..to?)
}

/// `n` things, singular or plural: `1 byte`, `3 bytes`.
fn counted(count: usize, one: &str) -> String {
    match count {
        | 0 => format!("no {one}s"),
        | 1 => format!("1 {one}"),
        | _ => format!("{count} {one}s"),
    }
}

/// `n` things with an irregular plural: `1 load or store`, `2 loads or
/// stores`.
fn counted_as(count: usize, one: &str, many: &str) -> String {
    match count {
        | 0 => format!("no {many}"),
        | 1 => format!("1 {one}"),
        | _ => format!("{count} {many}"),
    }
}

/// `a`, `a and b`, `a, b and c`.
fn list(items: &[String]) -> String {
    match items {
        | [] => String::new(),
        | [one] => one.clone(),
        | [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// A function list in words, at most four named.
fn names(callees: &[String]) -> String {
    let named: Vec<String> = callees
        .iter()
        .take(4)
        .map(|callee| format!("`{callee}`"))
        .collect();
    if callees.len() > named.len() {
        format!(
            "{} and {} more",
            named.join(", "),
            callees.len() - named.len()
        )
    } else {
        list(&named)
    }
}

fn sentence_case(text: &str) -> String {
    let mut characters = text.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    })
}

/// C99's keywords (§6.4.1p1), which phase 3 still reads as identifiers.
const KEYWORDS: [&str; 37] = [
    "auto",
    "break",
    "case",
    "char",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extern",
    "float",
    "for",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "register",
    "restrict",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "switch",
    "typedef",
    "union",
    "unsigned",
    "void",
    "volatile",
    "while",
    "_Bool",
    "_Complex",
    "_Imaginary",
];
