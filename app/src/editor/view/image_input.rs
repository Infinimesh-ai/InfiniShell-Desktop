//! 图片选择与预处理的输入归属，防止旧回调写入新的任务草稿。

use std::fmt;

use uuid::Uuid;
use warpui::{AppContext, EntityId, SingletonEntity, ViewContext};

use super::{EditorView, Event};
use crate::ai::agent::ImageContext;
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorImageInputScope {
    generation: Uuid,
    cli_generation: Option<Uuid>,
}

pub(super) struct ImageInputState {
    generation: Uuid,
    terminal_view_id: Option<EntityId>,
    external_consumer: bool,
    pending: Option<Uuid>,
}

impl Default for ImageInputState {
    fn default() -> Self {
        Self {
            generation: Uuid::new_v4(),
            terminal_view_id: None,
            external_consumer: false,
            pending: None,
        }
    }
}

pub struct ProcessedImageAttachments(pub Vec<ImageContext>);

impl fmt::Debug for ProcessedImageAttachments {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessedImageAttachments")
            .field("count", &self.0.len())
            .finish()
    }
}

impl EditorView {
    pub(crate) fn with_cli_image_input_owner(mut self, terminal_view_id: EntityId) -> Self {
        self.image_input.terminal_view_id = Some(terminal_view_id);
        self
    }

    pub(crate) fn set_external_image_input_generation(
        &mut self,
        generation: Uuid,
        ctx: &mut ViewContext<Self>,
    ) {
        self.abort_attached_images_future_handle(ctx);
        self.image_input.generation = generation;
        self.image_input.external_consumer = true;
        self.image_input.terminal_view_id = None;
    }

    pub(super) fn image_input_scope(&self, ctx: &AppContext) -> EditorImageInputScope {
        EditorImageInputScope {
            generation: self.image_input.generation,
            cli_generation: self
                .image_input
                .terminal_view_id
                .and_then(|view_id| CLIAgentSessionsModel::as_ref(ctx).input_generation(view_id)),
        }
    }

    pub(super) fn image_input_scope_matches(
        &self,
        scope: EditorImageInputScope,
        ctx: &AppContext,
    ) -> bool {
        self.image_input_scope(ctx) == scope
    }

    pub(super) fn begin_image_processing(
        &mut self,
        ctx: &mut ViewContext<Self>,
    ) -> Option<(Uuid, EditorImageInputScope)> {
        if self.image_input.pending.is_some() {
            return None;
        }
        let token = Uuid::new_v4();
        self.image_input.pending = Some(token);
        ctx.emit(Event::ProcessingAttachedImages(true));
        Some((token, self.image_input_scope(ctx)))
    }

    pub(super) fn image_processing_matches(
        &self,
        token: Uuid,
        scope: EditorImageInputScope,
        ctx: &AppContext,
    ) -> bool {
        self.image_input.pending == Some(token) && self.image_input_scope_matches(scope, ctx)
    }

    pub(super) fn finish_image_processing(&mut self, token: Uuid, ctx: &mut ViewContext<Self>) {
        if self.image_input.pending == Some(token) {
            self.image_input.pending = None;
            self.process_attached_images_future_handle = None;
            ctx.emit(Event::ProcessingAttachedImages(false));
        }
    }

    pub(super) fn invalidate_image_processing(&mut self) {
        self.image_input.pending = None;
        self.image_input.generation = Uuid::new_v4();
    }

    pub(super) fn strict_image_batch(&self, ctx: &AppContext) -> bool {
        self.image_input.external_consumer || self.image_input_scope(ctx).cli_generation.is_some()
    }

    pub(super) fn emit_processed_images(
        &self,
        images: Vec<ImageContext>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.image_input.external_consumer {
            ctx.emit(Event::ImagesProcessed {
                generation: self.image_input.generation,
                images: ProcessedImageAttachments(images),
            });
        }
    }

    pub(super) fn emit_selected_file_paths(
        &self,
        file_paths: Vec<String>,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.image_input.external_consumer {
            ctx.emit(Event::FilePathsSelected {
                generation: self.image_input.generation,
                file_paths,
            });
        }
    }
}

#[cfg(test)]
#[path = "image_input_tests.rs"]
mod tests;
