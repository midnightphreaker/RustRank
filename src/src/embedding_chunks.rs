use std::cmp::Reverse;

use crate::context::{DefKind, Definition};

const MAX_CHUNK_BYTES: usize = 4096;
const MAX_CHUNK_LINES: usize = 80;

#[derive(Debug, PartialEq, Eq)]
pub struct SourceChunk {
    pub start_line: usize,
    pub end_line: usize,
    pub symbol: Option<String>,
    pub content: String,
}

/// Split source along function boundaries, then bound every section by lines and bytes.
pub fn source_chunks(content: &str, defs: &[Definition]) -> Vec<SourceChunk> {
    let lines: Vec<_> = content.split_inclusive('\n').collect();
    let functions: Vec<_> = defs
        .iter()
        .filter(|def| def.kind == DefKind::Func && def.line > 0 && def.end_line >= def.line)
        .collect();
    let mut chunks = Vec::new();
    let mut section_start = 0;
    let mut section_owner: Option<usize> = None;
    for (index, _) in lines.iter().enumerate() {
        let line = index + 1;
        let owner = functions
            .iter()
            .enumerate()
            .filter(|(_, def)| def.line <= line && line <= def.end_line)
            .min_by_key(|(_, def)| (def.end_line - def.line, Reverse(def.line), &def.name))
            .map(|(index, _)| index);
        if owner != section_owner {
            append_section(
                &mut chunks,
                &lines[section_start..index],
                section_start + 1,
                section_owner.map(|owner| functions[owner].name.as_str()),
            );
            section_start = index;
            section_owner = owner;
        }
    }
    append_section(
        &mut chunks,
        &lines[section_start..],
        section_start + 1,
        section_owner.map(|owner| functions[owner].name.as_str()),
    );
    chunks
}

fn append_section(
    chunks: &mut Vec<SourceChunk>,
    lines: &[&str],
    section_start: usize,
    symbol: Option<&str>,
) {
    let mut content = String::new();
    let mut start_line = section_start;
    let mut end_line = section_start;
    for (offset, line) in lines.iter().enumerate() {
        let line_number = section_start + offset;
        if !content.is_empty()
            && (line_number - start_line >= MAX_CHUNK_LINES
                || content.len() + line.len() > MAX_CHUNK_BYTES)
        {
            push_chunk(chunks, &content, start_line, end_line, symbol);
            content.clear();
        }
        let mut remaining = *line;
        while remaining.len() > MAX_CHUNK_BYTES {
            let mut boundary = MAX_CHUNK_BYTES;
            while !remaining.is_char_boundary(boundary) {
                boundary -= 1;
            }
            push_chunk(
                chunks,
                &remaining[..boundary],
                line_number,
                line_number,
                symbol,
            );
            remaining = &remaining[boundary..];
        }
        if content.is_empty() {
            start_line = line_number;
        }
        content.push_str(remaining);
        end_line = line_number;
    }
    push_chunk(chunks, &content, start_line, end_line, symbol);
}

fn push_chunk(
    chunks: &mut Vec<SourceChunk>,
    content: &str,
    start_line: usize,
    end_line: usize,
    symbol: Option<&str>,
) {
    if !content.trim().is_empty() {
        chunks.push(SourceChunk {
            start_line,
            end_line,
            symbol: symbol.map(str::to_owned),
            content: content.to_owned(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::DefKind;

    fn function(name: &str, line: usize, end_line: usize) -> Definition {
        Definition {
            name: name.to_owned(),
            kind: DefKind::Func,
            line,
            end_line,
            has_args: false,
        }
    }

    #[test]
    fn separates_functions_and_preserves_module_text() {
        let source = "use example;\n\nfn first() {\n    one();\n}\n// between\nfn second() {}\nconst LAST: u8 = 1;\n";
        let defs = [
            function("first", 3, 5),
            function("second", 7, 7),
            Definition {
                kind: DefKind::Class,
                ..function("container", 1, 8)
            },
        ];
        let chunks = source_chunks(source, &defs);
        let spans: Vec<_> = chunks
            .iter()
            .map(|chunk| (chunk.start_line, chunk.end_line, chunk.symbol.as_deref()))
            .collect();
        assert_eq!(
            spans,
            [
                (1, 2, None),
                (3, 5, Some("first")),
                (6, 6, None),
                (7, 7, Some("second")),
                (8, 8, None)
            ]
        );
        assert_eq!(chunks[1].content, "fn first() {\n    one();\n}\n");
        assert_eq!(chunks[3].content, "fn second() {}\n");
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.content.as_str())
                .collect::<String>(),
            source
        );
    }

    #[test]
    fn attributes_nested_lines_to_the_innermost_function() {
        let source = "def outer():\n    def inner():\n        pass\n    return inner\n";
        let defs = [function("inner", 2, 3), function("outer", 1, 4)];
        let chunks = source_chunks(source, &defs);
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| (chunk.start_line, chunk.end_line, chunk.symbol.as_deref()))
                .collect::<Vec<_>>(),
            [
                (1, 1, Some("outer")),
                (2, 3, Some("inner")),
                (4, 4, Some("outer"))
            ]
        );
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.content.as_str())
                .collect::<String>(),
            source
        );
    }

    #[test]
    fn splits_large_sections_and_unicode_lines_without_losing_source() {
        let source = format!("{}>{}\nend", "call();\r\n".repeat(81), "🦀".repeat(2200));
        let chunks = source_chunks(&source, &[function("large", 1, 83)]);
        assert!(chunks.len() >= 5);
        assert_eq!((chunks[0].start_line, chunks[0].end_line), (1, 80));
        assert!(chunks.iter().all(|chunk| chunk.content.len() <= 4096
            && chunk.end_line - chunk.start_line < 80
            && chunk.symbol.as_deref() == Some("large")));
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.content.as_str())
                .collect::<String>(),
            source
        );
        assert_eq!(source_chunks(&source, &[function("large", 1, 83)]), chunks);
    }

    #[test]
    fn omits_whitespace_only_chunks_and_ignores_invalid_function_spans() {
        assert!(source_chunks(" \n\t\n", &[]).is_empty());
        assert!(source_chunks("", &[]).is_empty());
        let defs = [
            function("invalid", 0, 0),
            function("reversed", 2, 1),
            function("outside", 10, 20),
        ];
        let chunks = source_chunks("value", &defs);
        assert_eq!(
            chunks,
            [SourceChunk {
                start_line: 1,
                end_line: 1,
                symbol: None,
                content: "value".to_owned()
            }]
        );
    }
}
