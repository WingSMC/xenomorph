use crate::formatter::format_xenomorph;
use std::collections::HashMap;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use xenomorph_common::{
    lexer::{Token, TokenVariant},
    module::{
        types::{DeclarationInfo, ModuleDiagnostic},
        XenoRegistry,
    },
    parser::Declaration,
    semantic::{XenoAnnotation, XenoConstraint, XenoParent, XenoTrait, XenoTraitKind, XenoType},
    TokenData,
};
use xenomorph_lsp_common::types::{
    create_annotation_completion_item, create_completion_item, create_type_completion_item,
    BUILTIN_ANNOTATION_COMPLETIONS, BUILTIN_TYPE_COMPLETIONS,
};

mod formatter;
mod hover;
mod semantic_tokens;

struct HoverTarget {
    name: String,
    type_arguments: Vec<String>,
    range: Range,
}

#[derive(Clone, Copy)]
enum CompletionFrame<'src> {
    Annotation {
        name: &'src str,
        parameter_index: usize,
    },
    Parenthesis,
    Bracket,
    Curly,
    Angle,
}

fn token_starts_before(token: &Token<'_>, position: Position) -> bool {
    token.1.l < position.line || (token.1.l == position.line && token.1.c < position.character)
}

fn annotation_argument_context<'src>(
    tokens: &'src [Token<'src>],
    position: Position,
) -> Option<(&'src str, usize)> {
    let mut frames = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if !token_starts_before(token, position) {
            break;
        }

        match token.0 {
            TokenVariant::LParen => {
                let annotation_name = index
                    .checked_sub(1)
                    .and_then(|index| tokens.get(index))
                    .filter(|token| token.0 == TokenVariant::Identifier)
                    .and_then(|name| {
                        index
                            .checked_sub(2)
                            .and_then(|index| tokens.get(index))
                            .filter(|token| token.0 == TokenVariant::At)
                            .map(|_| name.1.v)
                    });
                frames.push(match annotation_name {
                    Some(name) => CompletionFrame::Annotation {
                        name,
                        parameter_index: 0,
                    },
                    None => CompletionFrame::Parenthesis,
                });
            }
            TokenVariant::LBracket => frames.push(CompletionFrame::Bracket),
            TokenVariant::LCurly => frames.push(CompletionFrame::Curly),
            TokenVariant::Lt => frames.push(CompletionFrame::Angle),
            TokenVariant::RParen => pop_frame(&mut frames, |frame| {
                matches!(
                    frame,
                    CompletionFrame::Annotation { .. } | CompletionFrame::Parenthesis
                )
            }),
            TokenVariant::RBracket => pop_frame(&mut frames, |frame| {
                matches!(frame, CompletionFrame::Bracket)
            }),
            TokenVariant::RCurly => {
                pop_frame(&mut frames, |frame| matches!(frame, CompletionFrame::Curly))
            }
            TokenVariant::Gt => {
                pop_frame(&mut frames, |frame| matches!(frame, CompletionFrame::Angle))
            }
            TokenVariant::Comma => {
                if let Some(CompletionFrame::Annotation {
                    parameter_index, ..
                }) = frames.last_mut()
                {
                    *parameter_index += 1;
                }
            }
            _ => {}
        }
    }

    frames.iter().rev().find_map(|frame| match frame {
        CompletionFrame::Annotation {
            name,
            parameter_index,
        } => Some((*name, *parameter_index)),
        _ => None,
    })
}

fn generic_constraint_context(tokens: &[Token<'_>], position: Position) -> bool {
    let mut angle_depth = 0usize;
    let mut constraint_at_depth = None;
    for token in tokens {
        if !token_starts_before(token, position) {
            break;
        }
        match token.0 {
            TokenVariant::Lt => angle_depth += 1,
            TokenVariant::Gt => {
                if constraint_at_depth == Some(angle_depth) {
                    constraint_at_depth = None;
                }
                angle_depth = angle_depth.saturating_sub(1);
            }
            TokenVariant::Colon if angle_depth > 0 => constraint_at_depth = Some(angle_depth),
            TokenVariant::Comma if constraint_at_depth == Some(angle_depth) => {
                constraint_at_depth = None
            }
            _ => {}
        }
    }
    constraint_at_depth == Some(angle_depth) && angle_depth > 0
}

fn pop_frame(frames: &mut Vec<CompletionFrame<'_>>, matches: impl Fn(CompletionFrame<'_>) -> bool) {
    if let Some(index) = frames.iter().rposition(|frame| matches(*frame)) {
        frames.truncate(index);
    }
}

fn deduplicate_completions(mut items: Vec<CompletionItem>) -> Vec<CompletionItem> {
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| seen.insert(item.label.clone()));
    items
}

struct Backend {
    client: Client,
    registry: XenoRegistry,
}

trait EditorPosition {
    fn to_editor_position(&self) -> Position;
    fn to_editor_range(&self) -> Range;
}
impl<'src> EditorPosition for TokenData<'src> {
    fn to_editor_position(&self) -> Position {
        Position {
            line: self.l,
            character: self.c,
        }
    }
    fn to_editor_range(&self) -> Range {
        let start = self.to_editor_position();
        Range {
            start,
            end: Position {
                line: start.line,
                character: start.character + self.v.len() as u32,
            },
        }
    }
}

fn diagnostics_for_module(errors: &[ModuleDiagnostic], module_path: &str) -> Vec<Diagnostic> {
    errors
        .iter()
        .filter(|error| error.module_path == module_path)
        .filter_map(|error| {
            let (line, col, len) = error.location?;
            Some(Diagnostic {
                range: Range {
                    start: Position {
                        line,
                        character: col,
                    },
                    end: Position {
                        line,
                        character: col + len,
                    },
                },
                severity: Some(match error.severity {
                    xenomorph_common::XenoDiagSeverity::Err => DiagnosticSeverity::ERROR,
                    xenomorph_common::XenoDiagSeverity::Warn => DiagnosticSeverity::WARNING,
                    xenomorph_common::XenoDiagSeverity::Info => DiagnosticSeverity::INFORMATION,
                }),
                message: error.message.clone(),
                source: Some("xenomorph".to_string()),
                ..Default::default()
            })
        })
        .collect()
}

impl Backend {
    // ── Path helpers ─────────────────────────────────────────────────

    /// Converts a file URI to a module path via the registry.
    fn uri_to_module_path(&self, uri: &Url) -> Option<String> {
        let file_path = uri.to_file_path().ok()?;
        self.registry.abs_path_to_module_path(&file_path)
    }

    // ── Completion helpers ──────────────────────────────────────────

    fn get_builtin_types(&self) -> impl Iterator<Item = CompletionItem> + '_ {
        self.registry
            .plugins
            .iter()
            .filter_map(|p| p.provide_types.map(|f| f()))
            .flatten()
            .map(|semantic_type| create_type_completion_item(semantic_type))
            .chain(BUILTIN_TYPE_COMPLETIONS.iter().cloned())
    }

    fn get_builtin_annotations(&self) -> impl Iterator<Item = CompletionItem> + '_ {
        self.registry
            .plugins
            .iter()
            .filter_map(|p| p.provide_annotations.map(|f| f()))
            .flatten()
            .map(|annotation| create_annotation_completion_item(annotation))
            .chain(BUILTIN_ANNOTATION_COMPLETIONS.iter().cloned())
    }

    fn find_annotation(&self, name: &str) -> Option<&'static XenoAnnotation> {
        self.registry
            .plugins
            .iter()
            .filter_map(|plugin| plugin.provide_annotations.map(|provide| provide()))
            .flatten()
            .copied()
            .chain(
                xenomorph_common::semantic::BUILTIN_ANNOTATIONS
                    .iter()
                    .copied(),
            )
            .find(|annotation| annotation.name == name)
    }

    fn semantic_types(&self) -> impl Iterator<Item = &'static XenoType> + '_ {
        self.registry
            .plugins
            .iter()
            .filter_map(|plugin| plugin.provide_types.map(|provide| provide()))
            .flatten()
            .copied()
            .chain(xenomorph_common::semantic::BUILTIN_TYPES.iter().copied())
    }

    fn semantic_traits(&self) -> Vec<&'static XenoTrait> {
        fn collect_trait(
            xeno_trait: &'static XenoTrait,
            traits: &mut Vec<&'static XenoTrait>,
            seen: &mut std::collections::HashSet<&'static str>,
        ) {
            if !seen.insert(xeno_trait.name) {
                return;
            }
            traits.push(xeno_trait);
            for parent in xeno_trait.parents.unwrap_or(&[]) {
                collect_trait(parent, traits, seen);
            }
        }

        let mut traits = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for xeno_trait in xenomorph_common::semantic::BUILTIN_TRAITS {
            collect_trait(xeno_trait, &mut traits, &mut seen);
        }
        for semantic_type in self.semantic_types() {
            for parent in semantic_type.parents.unwrap_or(&[]) {
                if let XenoParent::Trait(xeno_trait) = parent {
                    collect_trait(xeno_trait, &mut traits, &mut seen);
                }
            }
            for parameter in semantic_type.generic_params.unwrap_or(&[]) {
                if let Some(XenoConstraint::Trait(xeno_trait)) = parameter.constraint {
                    collect_trait(xeno_trait, &mut traits, &mut seen);
                }
            }
        }
        for annotation in self
            .registry
            .plugins
            .iter()
            .filter_map(|plugin| plugin.provide_annotations.map(|provide| provide()))
            .flatten()
            .copied()
            .chain(
                xenomorph_common::semantic::BUILTIN_ANNOTATIONS
                    .iter()
                    .copied(),
            )
        {
            for parameter in annotation.params {
                if let XenoConstraint::Trait(required) = parameter.constraint {
                    collect_trait(required, &mut traits, &mut seen);
                }
            }
        }
        traits
    }

    fn literal_completions(&self, required: &XenoTrait) -> Vec<CompletionItem> {
        let accepts = |name: &str| {
            required.kind == XenoTraitKind::Literal
                || self
                    .semantic_types()
                    .find(|candidate| candidate.name == name)
                    .is_some_and(|candidate| candidate.implements(required))
        };
        let mut items = Vec::new();
        if accepts("integer") || accepts("number") {
            items.push(create_completion_item(
                "0",
                Some("Integer literal"),
                CompletionItemKind::VALUE,
            ));
        }
        if accepts("number") {
            items.push(create_completion_item(
                "0.0",
                Some("Number literal"),
                CompletionItemKind::VALUE,
            ));
        }
        if accepts("string") {
            let mut item = create_completion_item(
                "string literal",
                Some("String literal"),
                CompletionItemKind::VALUE,
            );
            item.insert_text = Some("\"${1:value}\"".to_string());
            item.insert_text_format = Some(InsertTextFormat::SNIPPET);
            items.push(item);
        }
        if accepts("bool") {
            items.push(create_completion_item(
                "true",
                Some("Boolean literal"),
                CompletionItemKind::VALUE,
            ));
            items.push(create_completion_item(
                "false",
                Some("Boolean literal"),
                CompletionItemKind::VALUE,
            ));
        }
        items
    }

    fn regex_literal_completion() -> CompletionItem {
        let mut item = create_completion_item(
            "regex literal",
            Some("Regular-expression literal"),
            CompletionItemKind::VALUE,
        );
        item.insert_text = Some("/${1:pattern}/".to_string());
        item.insert_text_format = Some(InsertTextFormat::SNIPPET);
        item
    }

    fn completions_for_constraint(
        &self,
        required: XenoConstraint,
        module_path: Option<&str>,
    ) -> Vec<CompletionItem> {
        let all_types = || {
            let mut types: Vec<_> = self.get_builtin_types().collect();
            if let Some(module_path) = module_path {
                types.extend(self.get_module_completions(module_path));
            }
            deduplicate_completions(types)
        };

        let XenoConstraint::Trait(required) = required else {
            return all_types();
        };

        match required.kind {
            XenoTraitKind::Semantic => {
                let allowed: std::collections::HashSet<_> = self
                    .semantic_types()
                    .filter(|semantic_type| semantic_type.implements(required))
                    .map(|semantic_type| semantic_type.name)
                    .collect();
                all_types()
                    .into_iter()
                    .filter(|item| allowed.contains(item.label.as_str()))
                    .collect()
            }
            XenoTraitKind::Literal | XenoTraitKind::LiteralType => {
                self.literal_completions(required)
            }
            XenoTraitKind::RegexLiteral => vec![Self::regex_literal_completion()],
            XenoTraitKind::Identifier | XenoTraitKind::Type => all_types(),
            XenoTraitKind::Annotation => self.get_builtin_annotations().collect(),
            XenoTraitKind::Expression => {
                let mut items = all_types();
                items.extend(self.get_builtin_annotations());
                items.extend(self.literal_completions(&xenomorph_common::semantic::LITERAL));
                deduplicate_completions(items)
            }
        }
    }

    /// Returns completion items for all declarations visible from the given module
    /// (its own declarations + declarations from imported modules).
    fn get_module_completions(&self, module_path: &str) -> Vec<CompletionItem> {
        self.registry
            .get_all_declarations_in_scope(module_path)
            .into_iter()
            .map(|info| {
                let mut item = create_completion_item(
                    &info.name,
                    info.docs.as_deref(),
                    CompletionItemKind::CLASS,
                );
                if info.module_path != module_path {
                    item.detail = Some(format!(
                        "{} (from {})",
                        item.detail.unwrap_or_default(),
                        info.module_path
                    ));
                }
                item
            })
            .collect()
    }

    /// Returns completion items for import path suggestions.
    fn get_import_completions(&self, path_so_far: &str) -> Vec<CompletionItem> {
        self.registry
            .suggest_import(path_so_far)
            .into_iter()
            .map(|(name, _, is_dir)| {
                let kind = if is_dir {
                    CompletionItemKind::FOLDER
                } else {
                    CompletionItemKind::MODULE
                };
                CompletionItem {
                    label: name.clone(),
                    kind: Some(kind),
                    detail: Some(if is_dir {
                        "directory".to_string()
                    } else {
                        "module".to_string()
                    }),
                    // For directories, append / to keep completing
                    insert_text: if is_dir {
                        Some(format!("{}/", name))
                    } else {
                        None
                    },
                    // Retrigger completion after inserting a directory
                    command: if is_dir {
                        Some(Command {
                            title: "Trigger completion".to_string(),
                            command: "editor.action.triggerSuggest".to_string(),
                            arguments: None,
                        })
                    } else {
                        None
                    },
                    ..Default::default()
                }
            })
            .collect()
    }

    // ── Token helpers ───────────────────────────────────────────────

    fn find_token_at_position<'a>(
        tokens: &'a [Token<'a>],
        position: Position,
    ) -> Option<&'a Token<'a>> {
        tokens.iter().find(|(_, data)| {
            let token_range = data.to_editor_range();
            token_range.start <= position && position < token_range.end
        })
    }

    fn find_token_before_or_at_position<'a>(
        tokens: &'a [Token<'a>],
        position: Position,
    ) -> Option<&'a Token<'a>> {
        Self::find_token_at_position(tokens, position).or_else(|| {
            tokens.iter().rev().find(|(_, data)| {
                let end = data.to_editor_range().end;
                end.line < position.line
                    || (end.line == position.line && end.character <= position.character)
            })
        })
    }

    // ── Document validation ─────────────────────────────────────────

    /// Reloads the module in the registry from the given source text,
    /// then revalidates and publishes diagnostics for all of its importers.
    async fn validate_document(&self, uri: &Url, source: String) {
        let file_path = match uri.to_file_path() {
            Ok(p) => p,
            Err(_) => return,
        };
        let Some(module_path) = self.registry.abs_path_to_module_path(&file_path) else {
            return;
        };

        let errors = self.registry.load_module_from_source(&file_path, source);
        self.publish_module_diagnostics(uri.clone(), &module_path, &errors)
            .await;

        for importer in self.registry.revalidate_importers(&module_path) {
            let Some(abs_path) = self.registry.with_module(&importer, |_, _, module| {
                module.borrow_abs_path().to_path_buf()
            }) else {
                continue;
            };
            let Ok(importer_uri) = Url::from_file_path(abs_path) else {
                continue;
            };
            let importer_errors = self.registry.get_all_errors_for(&importer);
            self.publish_module_diagnostics(importer_uri, &importer, &importer_errors)
                .await;
        }
    }

    async fn publish_module_diagnostics(
        &self,
        uri: Url,
        module_path: &str,
        errors: &[ModuleDiagnostic],
    ) {
        let diagnostics = diagnostics_for_module(errors, module_path);
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }

    // ── Completions ─────────────────────────────────────────────────

    fn get_context_completions<'a>(
        &self,
        tokens: &[Token<'a>],
        _ast: &[Declaration<'a>],
        position: Position,
        module_path: Option<&str>,
    ) -> Vec<CompletionItem> {
        let mut items = Vec::new();

        if let Some((annotation_name, parameter_index)) =
            annotation_argument_context(tokens, position)
        {
            if let Some(parameter) = self
                .find_annotation(annotation_name)
                .and_then(|annotation| annotation.parameter_at(parameter_index))
            {
                return self.completions_for_constraint(parameter.constraint, module_path);
            }
            return items;
        }

        let add_top_level_snippets = |items: &mut Vec<CompletionItem>| {
            items.push(CompletionItem {
                label: "type".to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some("Declare a new type".to_string()),
                insert_text: Some("type ${1:Name} = ${0};".to_string()),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            });
            items.push(CompletionItem {
                label: "import".to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some("Import a module".to_string()),
                insert_text: Some("import ${1:module};".to_string()),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            });
        };

        let all_types = || -> Vec<CompletionItem> {
            let mut types: Vec<CompletionItem> = self.get_builtin_types().collect();
            if let Some(mp) = module_path {
                types.extend(self.get_module_completions(mp));
            }
            deduplicate_completions(types)
        };

        if generic_constraint_context(tokens, position) {
            items.extend(all_types());
            items.extend(self.semantic_traits().into_iter().map(|xeno_trait| {
                create_completion_item(
                    xeno_trait.name,
                    xeno_trait.documentation,
                    CompletionItemKind::INTERFACE,
                )
            }));
            return deduplicate_completions(items);
        }

        if let Some(current_token) = Self::find_token_before_or_at_position(tokens, position) {
            let client = self.client.clone();
            let msg = format!(
                "Current token: {:?} at line {}, col {}, value '{}'",
                current_token.0, current_token.1.l, current_token.1.c, current_token.1.v
            );
            tokio::spawn(async move {
                let _ = client.log_message(MessageType::INFO, msg).await;
            });

            match current_token.0 {
                TokenVariant::At => {
                    items.extend(self.get_builtin_annotations());
                }
                TokenVariant::Or | TokenVariant::Colon => {
                    items.extend(all_types());
                }
                TokenVariant::Eq => {
                    items.push(CompletionItem {
                        label: "struct".to_string(),
                        kind: Some(CompletionItemKind::SNIPPET),
                        detail: Some("Create a new struct type".to_string()),
                        insert_text: Some("{\n\t${1:property}: ${2:type},\n\t$0\n}".to_string()),
                        insert_text_format: Some(InsertTextFormat::SNIPPET),
                        ..Default::default()
                    });
                    items.push(CompletionItem {
                        label: "enum".to_string(),
                        kind: Some(CompletionItemKind::SNIPPET),
                        detail: Some("Create a new enum type".to_string()),
                        insert_text: Some(
                            "enum {\n\t${1:variant}: ${2:type},\n\t$0\n}".to_string(),
                        ),
                        insert_text_format: Some(InsertTextFormat::SNIPPET),
                        ..Default::default()
                    });
                    items.extend(all_types());
                }
                TokenVariant::Semicolon => {
                    add_top_level_snippets(&mut items);
                }
                TokenVariant::Import => {
                    items.extend(self.get_import_completions(""));
                }
                TokenVariant::Path => {
                    items.extend(self.get_import_completions(current_token.1.v));
                }
                TokenVariant::Identifier => {
                    let token_idx = tokens.iter().position(|t| {
                        t.1.l == current_token.1.l
                            && t.1.c == current_token.1.c
                            && t.1.v == current_token.1.v
                            && t.0 == current_token.0
                    });

                    let prev_variant = token_idx
                        .and_then(|idx| idx.checked_sub(1))
                        .and_then(|idx| tokens.get(idx))
                        .map(|t| t.0);

                    match prev_variant {
                        Some(TokenVariant::Colon) | Some(TokenVariant::Or) => {
                            items.extend(all_types());
                        }
                        _ => {
                            items.extend(self.get_builtin_annotations());
                        }
                    }
                }
                TokenVariant::RParen => {
                    items.extend(self.get_builtin_annotations());
                }
                TokenVariant::LCurly | TokenVariant::Comma => {
                    items.push(CompletionItem {
                        label: "property".to_string(),
                        kind: Some(CompletionItemKind::SNIPPET),
                        detail: Some("Add a property".to_string()),
                        insert_text: Some("${1:name}: ${2:type},".to_string()),
                        insert_text_format: Some(InsertTextFormat::SNIPPET),
                        ..Default::default()
                    });
                }
                _ => {
                    items.extend(all_types());
                    items.extend(self.get_builtin_annotations());
                }
            }
        } else {
            add_top_level_snippets(&mut items);
        }

        items
    }

    // ── Hover ───────────────────────────────────────────────────────

    fn hover_target_at_location(
        &self,
        tokens: &[Token],
        ast: &[Declaration],
        position: Position,
    ) -> Option<HoverTarget> {
        let token = Self::find_token_at_position(tokens, position)?;

        if token.0 != TokenVariant::Identifier {
            return None;
        }

        Some(HoverTarget {
            name: token.1.v.to_string(),
            type_arguments: hover::type_arguments_at_token(ast, &token.1).unwrap_or_default(),
            range: token.1.to_editor_range(),
        })
    }

    fn type_declaration_preview(
        &self,
        info: &DeclarationInfo,
        type_arguments: &[String],
    ) -> Option<String> {
        self.registry
            .with_module(&info.module_path, |_, ast, _| {
                ast.iter().find_map(|declaration| match declaration {
                    Declaration::Type { name, .. } if name.v == info.name => {
                        hover::format_type_declaration(declaration, type_arguments)
                    }
                    _ => None,
                })
            })
            .flatten()
    }

    fn user_type_hover(&self, target: &HoverTarget, current_module: &str) -> Option<Hover> {
        let info = self
            .registry
            .find_declaration(current_module, &target.name)?;
        let preview = self.type_declaration_preview(&info, &target.type_arguments)?;
        let mut contents = format!("**{}**", target.name);
        if info.module_path != current_module {
            contents.push_str(&format!(" *(from {})*", info.module_path));
        }
        if let Some(docs) = info.docs.as_deref().filter(|docs| !docs.is_empty()) {
            contents.push_str("\n\n");
            contents.push_str(docs);
        }
        contents.push_str("\n\n```xenomorph\n");
        contents.push_str(&preview);
        contents.push_str("\n```");

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: contents,
            }),
            range: Some(target.range),
        })
    }

    fn builtin_hover(&self, target: &HoverTarget) -> Option<Hover> {
        let builtin_info = self
            .get_builtin_types()
            .find(|item| item.label == target.name)
            .or_else(|| {
                self.get_builtin_annotations()
                    .find(|item| item.label == target.name)
            })?;

        let value = match builtin_info.documentation {
            Some(Documentation::MarkupContent(content)) => content.value,
            Some(Documentation::String(value)) => value,
            None => format!(
                "**{}**\n\n{}",
                builtin_info.label,
                builtin_info.detail.unwrap_or_default()
            ),
        };
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range: Some(target.range),
        })
    }

    // ── Goto Definition helpers ─────────────────────────────────────

    fn declaration_info_to_location(info: &DeclarationInfo) -> Option<Location> {
        let target_uri = Url::from_file_path(&info.abs_path).ok()?;
        Some(Location {
            uri: target_uri,
            range: Range {
                start: Position {
                    line: info.line,
                    character: info.column,
                },
                end: Position {
                    line: info.line,
                    character: info.column + info.name_len,
                },
            },
        })
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::FULL),
                        ..Default::default()
                    },
                )),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(true),
                    trigger_characters: Some(vec![
                        "|".to_string(),
                        ".".to_string(),
                        ":".to_string(),
                        "@".to_string(),
                        "(".to_string(),
                        ",".to_string(),
                        "{".to_string(),
                        "/".to_string(),
                        " ".to_string(),
                    ]),
                    all_commit_characters: None,
                    work_done_progress_options: Default::default(),
                    completion_item: None,
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                document_formatting_provider: Some(OneOf::Left(true)),
                document_symbol_provider: Some(OneOf::Left(true)),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            work_done_progress_options: Default::default(),
                            legend: semantic_tokens::legend(),
                            range: None,
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                        },
                    ),
                ),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        // Load the entry module and its transitive imports on startup
        let errors = self
            .registry
            .load_module(&[&self.registry.entry], true, None);
        for e in &errors {
            self.client
                .log_message(MessageType::WARNING, format!("Module error: {}", e))
                .await;
        }

        self.client
            .log_message(MessageType::INFO, "Xenomorph Language Server initialized!")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        self.validate_document(&uri, text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;

        // With TextDocumentSyncKind::FULL, last change contains the full text
        if let Some(change) = params.content_changes.into_iter().last() {
            self.validate_document(&uri, change.text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let module_path = self.uri_to_module_path(&uri);

        let completions =
            self.registry
                .with_module(module_path.as_deref().unwrap_or(""), |tokens, ast, _| {
                    self.get_context_completions(tokens, ast, position, module_path.as_deref())
                });

        Ok(Some(CompletionResponse::Array(
            completions.unwrap_or_default(),
        )))
    }

    async fn completion_resolve(&self, item: CompletionItem) -> Result<CompletionItem> {
        Ok(item)
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let module_path = self.uri_to_module_path(&uri);

        let current_module = module_path.as_deref().unwrap_or("");
        let target = self
            .registry
            .with_module(current_module, |tokens, ast, _| {
                self.hover_target_at_location(tokens, ast, position)
            })
            .flatten();
        let Some(target) = target else {
            return Ok(None);
        };

        Ok(self
            .user_type_hover(&target, current_module)
            .or_else(|| self.builtin_hover(&target)))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let module_path = self.uri_to_module_path(&params.text_document.uri);
        let tokens = self.registry.with_module(
            module_path.as_deref().unwrap_or(""),
            |tokens, ast, module| SemanticTokens {
                result_id: None,
                data: semantic_tokens::encode(module.borrow_source(), tokens, ast),
            },
        );

        Ok(tokens.map(SemanticTokensResult::Tokens))
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;
        let module_path = self.uri_to_module_path(&uri);

        let result =
            self.registry
                .with_module(module_path.as_deref().unwrap_or(""), |_, _, module| {
                    let source = module.borrow_source();
                    let formatted = format_xenomorph(source);

                    vec![TextEdit {
                        range: Range {
                            start: Position {
                                line: 0,
                                character: 0,
                            },
                            end: Position {
                                line: source.lines().count() as u32,
                                character: 0,
                            },
                        },
                        new_text: formatted,
                    }]
                });

        Ok(result)
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let module_path = self.uri_to_module_path(&uri);
        let mp = module_path.as_deref().unwrap_or("");

        // First try: local definition or import navigation
        let local_result = self.registry.with_module(mp, |tokens, ast, _| {
            let token = Self::find_token_at_position(tokens, position)?;

            // If cursor is on an import line, navigate to the imported file
            if token.0 == TokenVariant::Path {
                for decl in ast.iter() {
                    if let Declaration::Import { path, .. } = decl {
                        if path.join("/") == token.1.v {
                            let segments = path.to_vec();
                            if let Ok((_, abs_path)) = self.registry.resolve_import(&segments, None)
                            {
                                if abs_path.exists() {
                                    if let Ok(target_uri) = Url::from_file_path(&abs_path) {
                                        return Some(GotoDefinitionResponse::Scalar(Location {
                                            uri: target_uri,
                                            range: Range::default(),
                                        }));
                                    }
                                }
                            }
                            return None;
                        }
                    }
                }
            }

            // Try local declaration
            if token.0 == TokenVariant::Identifier {
                for decl in ast {
                    if let Declaration::Type { name, .. } = decl {
                        if name.v == token.1.v {
                            return Some(GotoDefinitionResponse::Scalar(Location {
                                uri: uri.clone(),
                                range: name.to_editor_range(),
                            }));
                        }
                    }
                }
            }

            None
        });

        if let Some(Some(response)) = local_result {
            return Ok(Some(response));
        }

        // Second try: cross-module declaration via the registry
        let cross_result = self.registry.with_module(mp, |tokens, _, _| {
            let token = Self::find_token_at_position(tokens, position)?;
            if token.0 != TokenVariant::Identifier {
                return None;
            }
            let info = self.registry.find_declaration(mp, token.1.v)?;
            Self::declaration_info_to_location(&info).map(GotoDefinitionResponse::Scalar)
        });

        Ok(cross_result.flatten())
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let module_path = self.uri_to_module_path(&uri);
        let current_module = module_path.as_deref().unwrap_or("");
        let include_declaration = params.context.include_declaration;

        let searched_name = self
            .registry
            .with_module(current_module, |tokens, _, _| {
                let token = Self::find_token_at_position(tokens, position)?;
                (token.0 == TokenVariant::Identifier).then(|| token.1.v.to_string())
            })
            .flatten();

        let Some(searched_name) = searched_name else {
            return Ok(None);
        };

        let Some(target) = self
            .registry
            .find_declaration(current_module, &searched_name)
        else {
            return Ok(None);
        };

        let module_paths: Vec<String> = self
            .registry
            .module_cache
            .read()
            .unwrap()
            .keys()
            .cloned()
            .collect();

        let mut locations = Vec::new();
        for module_path in module_paths {
            let Some(visible_decl) = self.registry.find_declaration(&module_path, &searched_name)
            else {
                continue;
            };

            if visible_decl.name != target.name || visible_decl.module_path != target.module_path {
                continue;
            }

            if let Some(mut module_locations) = self
                .registry
                .with_module(&module_path, |tokens, _, module| {
                    let uri = Url::from_file_path(module.borrow_abs_path()).ok()?;
                    Some(
                        tokens
                            .iter()
                            .filter_map(|token| {
                                if token.0 != TokenVariant::Identifier || token.1.v != searched_name
                                {
                                    return None;
                                }

                                if !include_declaration
                                    && module_path == target.module_path
                                    && token.1.l == target.line
                                    && token.1.c == target.column
                                {
                                    return None;
                                }

                                Some(Location {
                                    uri: uri.clone(),
                                    range: token.1.to_editor_range(),
                                })
                            })
                            .collect::<Vec<Location>>(),
                    )
                })
                .flatten()
            {
                locations.append(&mut module_locations);
            }
        }

        locations.sort_by(|left, right| {
            left.uri
                .as_str()
                .cmp(right.uri.as_str())
                .then_with(|| left.range.start.line.cmp(&right.range.start.line))
                .then_with(|| left.range.start.character.cmp(&right.range.start.character))
        });

        Ok(Some(locations))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let module_path = self.uri_to_module_path(&uri);

        let symbols =
            self.registry
                .with_module(module_path.as_deref().unwrap_or(""), |_, ast, _| {
                    #[allow(deprecated)]
                    ast.iter()
                        .filter_map(|decl| match decl {
                            Declaration::Import { .. } | Declaration::Custom { .. } => None,
                            Declaration::Type { name, .. } => Some(SymbolInformation {
                                name: name.v.to_string(),
                                kind: SymbolKind::STRUCT,
                                tags: None,
                                deprecated: None,
                                location: Location {
                                    uri: uri.clone(),
                                    range: name.to_editor_range(),
                                },
                                container_name: None,
                            }),
                        })
                        .collect::<Vec<SymbolInformation>>()
                });

        Ok(symbols.map(DocumentSymbolResponse::Flat))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = params.text_document.uri;
        let position = params.position;
        let module_path = self.uri_to_module_path(&uri);

        let result =
            self.registry
                .with_module(module_path.as_deref().unwrap_or(""), |tokens, ast, _| {
                    let token = Self::find_token_at_position(tokens, position)?;
                    if token.0 != TokenVariant::Identifier {
                        return None;
                    }

                    // Only allow renaming user-defined declarations
                    let is_user_defined = ast.iter().any(|decl| match decl {
                        Declaration::Import { .. } | Declaration::Custom { .. } => false,
                        Declaration::Type { name, .. } => name.v == token.1.v,
                    });

                    if !is_user_defined {
                        return None;
                    }

                    Some(PrepareRenameResponse::RangeWithPlaceholder {
                        range: token.1.to_editor_range(),
                        placeholder: token.1.v.to_string(),
                    })
                });

        Ok(result.flatten())
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let new_name = params.new_name;
        let module_path = self.uri_to_module_path(&uri);

        let result =
            self.registry
                .with_module(module_path.as_deref().unwrap_or(""), |tokens, ast, _| {
                    let token = Self::find_token_at_position(tokens, position)?;
                    if token.0 != TokenVariant::Identifier {
                        return None;
                    }

                    let old_name = token.1.v;

                    let is_user_defined = ast.iter().any(|decl| match decl {
                        Declaration::Import { .. } | Declaration::Custom { .. } => false,
                        Declaration::Type { name, .. } => name.v == old_name,
                    });

                    if !is_user_defined {
                        return None;
                    }

                    let edits: Vec<TextEdit> = tokens
                        .iter()
                        .filter(|t| t.0 == TokenVariant::Identifier && t.1.v == old_name)
                        .map(|t| TextEdit {
                            range: t.1.to_editor_range(),
                            new_text: new_name.clone(),
                        })
                        .collect();

                    if edits.is_empty() {
                        return None;
                    }

                    let mut changes = HashMap::new();
                    changes.insert(uri.clone(), edits);

                    Some(WorkspaceEdit {
                        changes: Some(changes),
                        ..Default::default()
                    })
                });

        Ok(result.flatten())
    }
}

#[tokio::main]
async fn main() {
    let reg = match XenoRegistry::new(false) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", e);
            return;
        }
    };

    let (service, socket) = LspService::new(move |client| Backend {
        client,
        registry: reg,
    });

    Server::new(tokio::io::stdin(), tokio::io::stdout(), socket)
        .serve(service)
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenomorph_common::lexer::Lexer;
    use xenomorph_common::module::types::ErrorPhase;
    use xenomorph_common::XenoDiagSeverity;

    fn context_at_end(source: &str) -> Option<(String, usize)> {
        let tokens = Lexer::tokenize(source).expect("completion fixture should lex");
        annotation_argument_context(
            &tokens,
            Position {
                line: 0,
                character: source.len() as u32,
            },
        )
        .map(|(name, index)| (name.to_string(), index))
    }

    #[test]
    fn annotation_context_tracks_variadic_parameter_index() {
        assert_eq!(
            context_at_end("@Lombok(Data, "),
            Some(("Lombok".to_string(), 1))
        );
    }

    #[test]
    fn annotation_context_ignores_nested_expression_commas() {
        assert_eq!(
            context_at_end("@Outer(@Inner(first, second), "),
            Some(("Outer".to_string(), 1))
        );
        assert_eq!(
            context_at_end("@Outer(@Inner(first, "),
            Some(("Inner".to_string(), 1))
        );
    }

    #[test]
    fn generic_constraint_context_only_matches_colons_inside_angle_parameters() {
        let generic = "type Box<T: Number";
        let generic_tokens = Lexer::tokenize(generic).expect("generic fixture should lex");
        assert!(generic_constraint_context(
            &generic_tokens,
            Position::new(0, generic.len() as u32)
        ));

        let field = "type Box = { value: Number";
        let field_tokens = Lexer::tokenize(field).expect("field fixture should lex");
        assert!(!generic_constraint_context(
            &field_tokens,
            Position::new(0, field.len() as u32)
        ));
    }

    #[test]
    fn match_argument_completion_inserts_regex_literal() {
        let completion = Backend::regex_literal_completion();

        assert_eq!(completion.label, "regex literal");
        assert_eq!(completion.insert_text.as_deref(), Some("/${1:pattern}/"));
        assert_eq!(
            completion.insert_text_format,
            Some(InsertTextFormat::SNIPPET)
        );
    }

    #[test]
    fn diagnostics_are_only_published_for_their_own_module() {
        let errors = vec![
            ModuleDiagnostic {
                module_path: "parser/p1".to_string(),
                message: "p1 error".to_string(),
                location: Some((1, 2, 3)),
                phase: ErrorPhase::Parser,
                severity: XenoDiagSeverity::Err,
            },
            ModuleDiagnostic {
                module_path: "parser/p2".to_string(),
                message: "p2 error".to_string(),
                location: Some((4, 5, 6)),
                phase: ErrorPhase::Analyzer,
                severity: XenoDiagSeverity::Err,
            },
        ];

        let diagnostics = diagnostics_for_module(&errors, "parser/p2");

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "p2 error");
        assert_eq!(diagnostics[0].range.start, Position::new(4, 5));
        assert_eq!(diagnostics[0].range.end, Position::new(4, 11));
    }
}
