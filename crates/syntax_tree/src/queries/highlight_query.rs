use std::cell::RefCell;
use std::collections::HashMap;
use std::iter;
use std::ops::Range;
use std::sync::Arc;

use arborium::tree_sitter::{Node, Parser, Query, QueryCursor, TextProvider, Tree};
use rangemap::RangeMap;
use streaming_iterator::StreamingIterator;
use string_offset::{ByteOffset, CharOffset};
use warp_editor::content::buffer::{Buffer, ToBufferByteOffset, ToBufferCharOffset};
use warp_editor::content::text::Bytes;
use warpui_core::color::ColorU;

thread_local! {
    static INJECTION_PARSER: RefCell<Parser> = RefCell::new(Parser::new());
}

/// Color mapping from parsed syntax token name to its corresponding highlighting color.
#[derive(Clone, Copy)]
pub struct ColorMap {
    pub keyword_color: ColorU,
    pub function_color: ColorU,
    pub string_color: ColorU,
    pub type_color: ColorU,
    pub number_color: ColorU,
    pub comment_color: ColorU,
    pub property_color: ColorU,
    pub tag_color: ColorU,
}

/// Query for retrieving syntax highlighting information on the tokens.
pub struct HighlightQuery {
    highlight_map: Vec<Option<ColorU>>,
}

pub struct InjectionHighlightQuery {
    pub language: Arc<languages::Language>,
    pub highlight_query: HighlightQuery,
}

impl HighlightQuery {
    pub fn new(query: &Query, color_map: ColorMap) -> Self {
        let highlight_map = query
            .capture_names()
            .iter()
            .map(|name| convert_capture_name_to_color(name, &color_map))
            .collect();

        Self { highlight_map }
    }

    /// Given the a character range, return its corresponding highlight colors.
    pub fn get_highlighted_chunks(
        &self,
        range: Range<CharOffset>,
        query: &Query,
        buffer: &Buffer,
        tree: &Tree,
    ) -> RangeMap<CharOffset, ColorU> {
        let mut range_map = RangeMap::new();

        let mut cursor = QueryCursor::new();
        let byte_start = range.start.to_buffer_byte_offset(buffer).as_usize();
        let byte_end = range.end.to_buffer_byte_offset(buffer).as_usize();
        cursor.set_byte_range(byte_start..byte_end);
        let mut captures = cursor.captures(query, tree.root_node(), TextBuffer(buffer));

        while let Some(matches) = captures.next() {
            for cap in matches.0.captures {
                let insertion_range = cap.node.byte_range();
                let color = self
                    .highlight_map
                    .get(cap.index as usize)
                    .and_then(|inner| *inner);

                if let Some(color) = color {
                    let char_start =
                        ByteOffset::from(insertion_range.start).to_buffer_char_offset(buffer);
                    let char_end =
                        ByteOffset::from(insertion_range.end).to_buffer_char_offset(buffer);
                    if char_start < char_end {
                        range_map.insert(char_start..char_end, color);
                    }
                }
            }
        }

        range_map
    }

    /// Process language injections and return highlights for injected regions.
    /// This handles embedded languages like JavaScript/CSS in Vue files.
    pub fn get_injection_highlights(
        &self,
        range: Range<CharOffset>,
        injections_query: &Query,
        injection_highlight_queries: &HashMap<String, InjectionHighlightQuery>,
        buffer: &Buffer,
        tree: &Tree,
    ) -> RangeMap<CharOffset, ColorU> {
        let mut range_map = RangeMap::new();

        let mut cursor = QueryCursor::new();
        let byte_start = range.start.to_buffer_byte_offset(buffer).as_usize();
        let byte_end = range.end.to_buffer_byte_offset(buffer).as_usize();
        cursor.set_byte_range(byte_start..byte_end);

        let injection_content_idx = injections_query
            .capture_names()
            .iter()
            .position(|name| *name == "injection.content")
            .map(|idx| idx as u32);
        let injection_language_idx = injections_query
            .capture_names()
            .iter()
            .position(|name| *name == "injection.language")
            .map(|idx| idx as u32);

        let mut matches = cursor.matches(injections_query, tree.root_node(), TextBuffer(buffer));

        while let Some(query_match) = matches.next() {
            // 中文注释：优先读取 query 的 #set! 属性，这是注入语言的权威来源。
            // Source: arborium-highlight-2.18.0/src/tree_sitter.rs
            let mut lang_name = injections_query
                .property_settings(query_match.pattern_index)
                .iter()
                .find(|prop| prop.key.as_ref() == "injection.language")
                .and_then(|prop| prop.value.as_deref())
                .map(str::to_owned);
            let mut content_node: Option<Node> = None;

            for cap in query_match.captures {
                if Some(cap.index) == injection_content_idx {
                    content_node = Some(cap.node);
                } else if Some(cap.index) == injection_language_idx && lang_name.is_none() {
                    let bytes = collect_buffer_bytes(
                        buffer,
                        ByteOffset::from(cap.node.start_byte()),
                        ByteOffset::from(cap.node.end_byte()),
                    );
                    if let Ok(value) = std::str::from_utf8(&bytes) {
                        if !value.is_empty() {
                            lang_name = Some(value.to_string());
                        }
                    }
                }
            }

            if let (Some(node), Some(lang_name)) = (content_node, lang_name) {
                let normalized_lang = normalize_injection_language_name(&lang_name);

                if let Some(injection_query) = injection_highlight_queries.get(normalized_lang) {
                    let content_range = node.byte_range();
                    let local_start = byte_start.saturating_sub(content_range.start);
                    let local_end = byte_end
                        .min(content_range.end)
                        .saturating_sub(content_range.start);

                    if local_start < local_end {
                        let source = collect_buffer_bytes(
                            buffer,
                            ByteOffset::from(content_range.start),
                            ByteOffset::from(content_range.end),
                        );

                        // 中文注释：注入片段必须先按目标语言单独建树，再把高亮偏移回原文件。
                        let highlights = injection_query
                            .highlight_query
                            .get_highlighted_chunks_for_injection(
                                local_start..local_end,
                                &injection_query.language,
                                &source,
                                content_range.start,
                                buffer,
                            );

                        for (highlight_range, color) in highlights.iter() {
                            range_map.insert(highlight_range.clone(), *color);
                        }
                    }
                }
            }
        }

        range_map
    }

    fn get_highlighted_chunks_for_injection(
        &self,
        local_byte_range: Range<usize>,
        language: &languages::Language,
        source: &[u8],
        base_byte_offset: usize,
        buffer: &Buffer,
    ) -> RangeMap<CharOffset, ColorU> {
        INJECTION_PARSER.with(|parser| {
            let mut parser = parser.borrow_mut();
            parser
                .set_language(&language.grammar)
                .expect("注入语言语法应当兼容 parser");

            let Some(tree) = parser.parse(source, None) else {
                return RangeMap::new();
            };

            let mut range_map = RangeMap::new();
            let mut cursor = QueryCursor::new();
            cursor.set_byte_range(local_byte_range);
            let mut captures = cursor.captures(
                &language.highlight_query,
                tree.root_node(),
                TextSlice(source),
            );

            while let Some(matches) = captures.next() {
                for cap in matches.0.captures {
                    let insertion_range = cap.node.byte_range();
                    let color = self
                        .highlight_map
                        .get(cap.index as usize)
                        .and_then(|inner| *inner);

                    if let Some(color) = color {
                        let global_start =
                            injection_byte_to_buffer_byte(base_byte_offset, insertion_range.start);
                        let global_end =
                            injection_byte_to_buffer_byte(base_byte_offset, insertion_range.end);
                        let char_start =
                            ByteOffset::from(global_start).to_buffer_char_offset(buffer);
                        let char_end = ByteOffset::from(global_end).to_buffer_char_offset(buffer);
                        if char_start < char_end {
                            range_map.insert(char_start..char_end, color);
                        }
                    }
                }
            }

            range_map
        })
    }
}

fn injection_byte_to_buffer_byte(base_byte_offset: usize, local_byte_offset: usize) -> usize {
    // 中文注释：注入片段来自已解析的 Vue tree-sitter 节点，映射回 Buffer 时需要抵消
    // Buffer/TreeSitter 边界上的 1-byte 偏移，避免 token 首字符漏高亮。
    base_byte_offset + local_byte_offset.saturating_sub(1)
}

fn normalize_injection_language_name(name: &str) -> &str {
    match name {
        "js" => "javascript",
        "ts" => "typescript",
        "less" | "postcss" => "scss",
        other => other,
    }
}

fn collect_buffer_bytes(buffer: &Buffer, start: ByteOffset, end: ByteOffset) -> Vec<u8> {
    let mut bytes = Vec::new();
    for chunk in buffer.bytes_in_range(start, end) {
        bytes.extend_from_slice(chunk);
    }
    bytes
}

fn convert_capture_name_to_color(name: &str, color_map: &ColorMap) -> Option<ColorU> {
    match name {
        "text.title" => return Some(color_map.keyword_color),
        "text.literal" => return Some(color_map.string_color),
        "text.uri" => return Some(color_map.function_color),
        "text.reference" => return Some(color_map.property_color),
        _ => {}
    }
    match name.split('.').next() {
        Some("keyword") => Some(color_map.keyword_color),
        Some("function") => Some(color_map.function_color),
        Some("string") => Some(color_map.string_color),
        Some("type") => Some(color_map.type_color),
        Some("number") => Some(color_map.number_color),
        Some("comment") => Some(color_map.comment_color),
        Some("property") => Some(color_map.property_color),
        Some("tag") => Some(color_map.tag_color),
        Some("punctuation") => Some(color_map.comment_color),
        _ => None,
    }
}

// The default tree-sitter implementation here is unsafe (since the cursor could query invalid ranges outside of content length).
// TODO(kevin): Once we migrate buffer to store ArrayStrings. We should implement the chunks API on buffer directly to avoid collecting
// into a String and then chunking them again for highlighting.
pub struct TextSlice<'a>(pub &'a [u8]);

impl TextSlice<'_> {
    fn get(&self, range: Range<usize>) -> Self {
        Self(self.0.get(range).unwrap_or_default())
    }
}

impl AsRef<[u8]> for TextSlice<'_> {
    fn as_ref(&self) -> &[u8] {
        self.0
    }
}

impl<'a> TextProvider<TextSlice<'a>> for TextSlice<'a> {
    type I = iter::Once<TextSlice<'a>>;

    fn text(&mut self, node: Node) -> Self::I {
        iter::once(self.get(node.byte_range()))
    }
}

pub struct TextBuffer<'a>(pub &'a Buffer);

impl<'a> TextProvider<&'a [u8]> for TextBuffer<'a> {
    type I = Bytes<'a>;

    fn text(&mut self, node: Node) -> Self::I {
        let range = node.range();
        self.0.bytes_in_range(
            ByteOffset::from(range.start_byte),
            ByteOffset::from(range.end_byte),
        )
    }
}
