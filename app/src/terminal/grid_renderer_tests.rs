use std::any::Any;
use std::collections::HashMap;
use std::iter::Empty;
use std::ops::RangeInclusive;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::Result;
use futures::FutureExt as _;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{Vector2F, Vector2I, vec2f};
use warp_core::features::FeatureFlag;
use warpui::assets::asset_cache::AssetCache;
use warpui::elements::{Point as ScreenPoint, Rect, ZIndex};
use warpui::event::DispatchedEvent;
use warpui::fonts::{Cache as FontCache, FamilyId, FontId, GlyphId, Metrics, Properties};
use warpui::image_cache::ImageType;
use warpui::platform::{self, LoadedSystemFonts, TextLayoutSystem};
use warpui::text_layout::{ClipConfig, Line, TextAlignment, TextFrame};
use warpui::units::{IntoLines, Lines, Pixels};
use warpui::{
    AfterLayoutContext, App, AppContext, Element, Entity, EntityIdSet, EventContext, LayoutContext,
    PaintContext, Presenter, SingletonEntity, SizeConstraint, TypedActionView, View,
    WindowInvalidation,
};

use super::{CachedBackgroundColor, active_or_next_match};
use crate::appearance::Appearance;
use crate::settings::EnforceMinimumContrast;
use crate::terminal::color;
use crate::terminal::grid_size_util::calculate_grid_baseline_position;
use crate::terminal::model::ObfuscateSecrets;
use crate::terminal::model::ansi::Handler;
use crate::terminal::model::grid::RespectDisplayedOutput;
use crate::terminal::model::grid::grid_handler::GridHandler;
use crate::terminal::model::image_map::StoredImageMetadata;
use crate::terminal::model::index::Point;
use crate::terminal::model::iterm_image::{ITermImage, ITermImageMetadata};
use crate::terminal::model::selection::SelectionPoint;
use crate::terminal::{SizeInfo, grid_renderer};

fn rect_from_points(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> RectF {
    RectF::from_points(vec2f(min_x, min_y), vec2f(max_x, max_y))
}

#[derive(Default)]
struct FontLookupCounts {
    select_font: AtomicUsize,
    glyph_for_char: AtomicUsize,
}

struct CountingFontDb {
    counts: Arc<FontLookupCounts>,
}

struct NoLoadedFonts;

impl LoadedSystemFonts for NoLoadedFonts {
    fn as_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

impl platform::FontDB for CountingFontDb {
    fn load_from_bytes(&mut self, _name: &str, _bytes: Vec<Vec<u8>>) -> Result<FamilyId> {
        Ok(FamilyId(0))
    }

    #[cfg(not(target_family = "wasm"))]
    fn load_from_system(&mut self, _font_family: &str) -> Result<FamilyId> {
        Ok(FamilyId(0))
    }

    #[cfg(not(target_family = "wasm"))]
    fn load_all_system_fonts(
        &self,
    ) -> futures::future::BoxFuture<'static, Box<dyn LoadedSystemFonts>> {
        futures::future::ready(Box::new(NoLoadedFonts) as Box<dyn LoadedSystemFonts>).boxed()
    }

    #[cfg(not(target_family = "wasm"))]
    fn process_loaded_system_fonts(
        &mut self,
        _loaded_system_fonts: Box<dyn LoadedSystemFonts>,
    ) -> Vec<(Option<FamilyId>, warpui::fonts::FontInfo)> {
        Vec::new()
    }

    fn family_id_for_name(&self, _name: &str) -> Option<FamilyId> {
        None
    }

    fn load_family_name_from_id(&self, _id: FamilyId) -> Option<String> {
        None
    }

    fn select_font(&self, _family_id: FamilyId, _properties: Properties) -> FontId {
        self.counts.select_font.fetch_add(1, Ordering::Relaxed);
        FontId(0)
    }

    fn fallback_fonts(&self, _character: char, _font_id: FontId) -> Vec<FontId> {
        Vec::new()
    }

    fn font_metrics(&self, _font_id: FontId) -> Metrics {
        Metrics {
            units_per_em: 2048,
            ascent: 1901,
            descent: -483,
            line_gap: 0,
        }
    }

    fn glyph_advance(&self, _font_id: FontId, _glyph_id: GlyphId) -> Result<Vector2I> {
        Ok(Vector2I::zero())
    }

    fn glyph_raster_bounds(
        &self,
        _font_id: FontId,
        _size: f32,
        _glyph_id: GlyphId,
        _scale: Vector2F,
        _glyph_config: &warpui::rendering::GlyphConfig,
    ) -> Result<pathfinder_geometry::rect::RectI> {
        Ok(pathfinder_geometry::rect::RectI::default())
    }

    fn glyph_typographic_bounds(
        &self,
        _font_id: FontId,
        _glyph_id: GlyphId,
    ) -> Result<pathfinder_geometry::rect::RectI> {
        Ok(pathfinder_geometry::rect::RectI::default())
    }

    fn rasterize_glyph(
        &self,
        _font_id: FontId,
        _size: f32,
        _glyph_id: GlyphId,
        _scale: Vector2F,
        _subpixel_alignment: warpui::fonts::SubpixelAlignment,
        _glyph_config: &warpui::rendering::GlyphConfig,
        _format: warpui::fonts::canvas::RasterFormat,
    ) -> Result<warpui::fonts::RasterizedGlyph> {
        Ok(warpui::fonts::RasterizedGlyph {
            canvas: warpui::fonts::canvas::Canvas {
                pixels: Vec::new(),
                size: pathfinder_geometry::vector::vec2i(0, 0),
                row_stride: 0,
                format: warpui::fonts::canvas::RasterFormat::Rgba32,
            },
            is_emoji: false,
        })
    }

    fn glyph_for_char(&self, _font_id: FontId, _char: char) -> Option<GlyphId> {
        self.counts.glyph_for_char.fetch_add(1, Ordering::Relaxed);
        Some(0)
    }

    fn text_layout_system(&self) -> &dyn TextLayoutSystem {
        self
    }
}

impl TextLayoutSystem for CountingFontDb {
    fn layout_line(
        &self,
        _text: &str,
        line_style: platform::LineStyle,
        _style_runs: &[(std::ops::Range<usize>, warpui::text_layout::StyleAndFont)],
        _max_width: f32,
        _clip_config: ClipConfig,
    ) -> Line {
        Line::empty(line_style.font_size, line_style.line_height_ratio, 0)
    }

    fn layout_text(
        &self,
        _text: &str,
        line_style: platform::LineStyle,
        _style_runs: &[(std::ops::Range<usize>, warpui::text_layout::StyleAndFont)],
        _max_width: f32,
        _max_height: f32,
        _alignment: TextAlignment,
        _first_line_head_indent: Option<f32>,
    ) -> TextFrame {
        TextFrame::empty(line_style.font_size, line_style.line_height_ratio)
    }
}

#[test]
fn fallback_font_family_for_char_skips_font_lookup_when_unconfigured() {
    let counts = Arc::new(FontLookupCounts::default());
    let font_cache = FontCache::new(Box::new(CountingFontDb {
        counts: Arc::clone(&counts),
    }));
    let builder =
        super::AttributedStringBuilder::new(FamilyId(0), FamilyId(0), None, &font_cache, 1);

    assert_eq!(builder.fallback_font_family_for_char('中'), None);
    assert_eq!(counts.select_font.load(Ordering::Relaxed), 0);
    assert_eq!(counts.glyph_for_char.load(Ordering::Relaxed), 0);
}

// TODO(CORE-2002): Make test non-Mac specific by switching to using bundled Roboto font.
#[test]
#[cfg_attr(
    not(target_os = "macos"),
    ignore = "Assumes existence of Arial font, which is only guaranteed on macOS"
)]
fn test_calculate_grid_baseline_position() {
    let font_db = warpui::platform::test::FontDB::new();
    let mut font_cache = FontCache::new(Box::new(font_db));
    // Note we've restricted this unit test to Mac, so we expect Arial to exist.
    let arial = font_cache
        .load_system_font("Arial")
        .expect("Arial must exist");
    let baseline_position = calculate_grid_baseline_position(
        &font_cache,
        arial,
        16., /* font_size */
        1.2, /* line_height_ratio */
        19., /* cell_size_y */
    );
    assert_eq!(baseline_position, vec2f(0., 15.));
}

#[test]
fn test_next_match_same_row_matches() {
    let match_1 = Point::new(0, 0)..=Point::new(0, 4);
    let match_2 = Point::new(1, 0)..=Point::new(1, 4);
    let matches = [match_1.clone(), match_2.clone()];
    let mut filter_match_iter = matches.iter();

    let mut current_match = None;

    // The first match should return for points (0,0) through (0,4).
    for i in 0..=4 {
        current_match =
            active_or_next_match(&mut filter_match_iter, current_match, &Point::new(0, i));
        assert_eq!(current_match, Some(&match_1));
    }

    // The second match should return for points (1,0) through (1,4).
    for i in 0..=4 {
        current_match =
            active_or_next_match(&mut filter_match_iter, current_match, &Point::new(1, i));
        assert_eq!(current_match, Some(&match_2));
    }

    // There should be no more matches left after we advance to point (2,0).
    current_match = active_or_next_match(&mut filter_match_iter, current_match, &Point::new(2, 0));
    assert_eq!(current_match, None);
}

#[test]
fn test_next_match_multi_row_matches() {
    let match_1 = Point::new(0, 0)..=Point::new(1, 2);
    let match_2 = Point::new(2, 0)..=Point::new(3, 2);
    let matches = [match_1.clone(), match_2.clone()];
    let mut match_iter = matches.iter();

    let mut current_match = None;

    // The first match should be returned for all points from (0,0) to (1,2).
    let points_1 = [
        Point::new(0, 0),
        Point::new(0, 1),
        Point::new(0, 2),
        Point::new(1, 0),
        Point::new(1, 1),
        Point::new(1, 2),
    ];
    for point in points_1.iter() {
        current_match = active_or_next_match(&mut match_iter, current_match, point);
        assert_eq!(current_match, Some(&match_1));
    }

    // The second match should be returned for all points from (2,0) to (3,2).
    let points_2 = [
        Point::new(2, 0),
        Point::new(2, 1),
        Point::new(2, 2),
        Point::new(3, 0),
        Point::new(3, 1),
        Point::new(3, 2),
    ];
    for point in points_2.iter() {
        current_match = active_or_next_match(&mut match_iter, current_match, point);
        assert_eq!(current_match, Some(&match_2));
    }

    // There should be no more matches left after we advance to point (4,0).
    current_match = active_or_next_match(&mut match_iter, current_match, &Point::new(4, 0));
    assert_eq!(current_match, None);
}

#[test]
fn test_active_or_next_match_point_before_next_match() {
    let match_1 = Point::new(1, 0)..=Point::new(1, 4);
    let match_2 = Point::new(3, 0)..=Point::new(3, 4);
    let matches = [match_1.clone(), match_2.clone()];
    let mut match_iter = matches.iter();

    // The match for (0,0) should be the first match.
    let mut current_match = active_or_next_match(&mut match_iter, None, &Point::new(0, 0));
    assert_eq!(current_match, Some(&match_1));

    // The match for (2,0) should be the second match.
    current_match = active_or_next_match(&mut match_iter, current_match, &Point::new(2, 0));
    assert_eq!(current_match, Some(&match_2));
}

#[test]
fn test_calculate_background_bounds() {
    let origin = vec2f(100., 100.);
    let cell_size = vec2f(2., 4.);
    let max_columns = 150;
    let create_cached = |start_row: usize, start_col: usize, end_row: usize, end_col: usize| {
        CachedBackgroundColor {
            start: SelectionPoint {
                row: start_row.into_lines(),
                col: start_col,
            },
            end: SelectionPoint {
                row: end_row.into_lines(),
                col: end_col,
            },
            background_color: Default::default(),
        }
    };

    // Background with 1 row
    let (start_row, start_col, end_row, end_col) = (10, 20, 10, 130);
    let cached = create_cached(start_row, start_col, end_row, end_col);
    assert_eq!(
        grid_renderer::calculate_background_bounds(origin, cached, cell_size, max_columns),
        vec![rect_from_points(
            origin.x() + (start_col as f32) * cell_size.x(),
            origin.y() + (start_row as f32) * cell_size.y(),
            origin.x() + (end_col as f32 + 1.) * cell_size.x(),
            origin.y() + (end_row as f32 + 1.) * cell_size.y()
        )]
    );

    // Background with 2 rows
    let (start_row, start_col, end_row, end_col) = (20, 30, 21, 100);
    let cached = create_cached(start_row, start_col, end_row, end_col);
    assert_eq!(
        grid_renderer::calculate_background_bounds(origin, cached, cell_size, max_columns),
        vec![
            rect_from_points(
                origin.x() + (start_col as f32) * cell_size.x(),
                origin.y() + (start_row as f32) * cell_size.y(),
                origin.x() + (max_columns as f32 + 1.) * cell_size.x(),
                origin.y() + (start_row as f32 + 1.) * cell_size.y()
            ),
            rect_from_points(
                origin.x(),
                origin.y() + (start_row as f32 + 1.) * cell_size.y(),
                origin.x() + (end_col as f32 + 1.) * cell_size.x(),
                origin.y() + (end_row as f32 + 1.) * cell_size.y()
            ),
        ]
    );

    // Background with 3+ rows
    let assert_multi_row_selection_bounds =
        |start_row: usize, start_col: usize, end_row: usize, end_col: usize| {
            let cached = create_cached(start_row, start_col, end_row, end_col);
            assert_eq!(
                grid_renderer::calculate_background_bounds(origin, cached, cell_size, max_columns),
                vec![
                    rect_from_points(
                        origin.x() + (start_col as f32) * cell_size.x(),
                        origin.y() + (start_row as f32) * cell_size.y(),
                        origin.x() + (max_columns as f32 + 1.) * cell_size.x(),
                        origin.y() + (start_row as f32 + 1.) * cell_size.y()
                    ),
                    rect_from_points(
                        origin.x(),
                        origin.y() + (start_row as f32 + 1.) * cell_size.y(),
                        origin.x() + (max_columns as f32 + 1.) * cell_size.x(),
                        origin.y() + (end_row as f32) * cell_size.y()
                    ),
                    rect_from_points(
                        origin.x(),
                        origin.y() + (end_row as f32) * cell_size.y(),
                        origin.x() + (end_col as f32 + 1.) * cell_size.x(),
                        origin.y() + (end_row as f32 + 1.) * cell_size.y()
                    ),
                ]
            );
        };
    assert_multi_row_selection_bounds(30, 80, 32, 40); // 3 lines
    assert_multi_row_selection_bounds(40, 60, 43, 10); // 4 lines
    assert_multi_row_selection_bounds(50, 140, 59, 20); // 10 lines
}

#[test]
fn test_calculate_selection_bounds() {
    let origin = vec2f(100., 100.);
    let size_info = SizeInfo::new(
        Vector2F::zero(),
        Pixels::new(2.),
        Pixels::new(4.),
        Pixels::new(8.),
        Pixels::new(16.),
    )
    .with_rows_and_columns(151, 151);

    let cell_width = size_info.cell_width_px.as_f32();
    let cell_height = size_info.cell_height_px.as_f32();
    let horizontal_padding = size_info.padding_x_px.as_f32();
    let max_columns = size_info.columns - 1;

    let make_selection_point = |row: usize, col: usize| SelectionPoint {
        row: row.into_lines(),
        col,
    };

    let start = make_selection_point(10, 10);
    let end = make_selection_point(20, 50);

    let assert_selection_bounds = |scroll_top: Lines| {
        assert_eq!(
            grid_renderer::calculate_selection_bounds(&start, &end, &size_info, scroll_top, origin),
            vec![
                rect_from_points(
                    origin.x() + horizontal_padding + (start.col as f32) * cell_width,
                    origin.y() + ((start.row - scroll_top).as_f64() as f32) * cell_height,
                    origin.x() + horizontal_padding + (max_columns as f32 + 1.) * cell_width,
                    origin.y() + ((start.row - scroll_top).as_f64() as f32 + 1.) * cell_height
                ),
                rect_from_points(
                    origin.x() + horizontal_padding,
                    origin.y() + ((start.row - scroll_top).as_f64() as f32 + 1.) * cell_height,
                    origin.x() + horizontal_padding + (max_columns as f32 + 1.) * cell_width,
                    origin.y() + ((end.row - scroll_top).as_f64() as f32) * cell_height
                ),
                rect_from_points(
                    origin.x() + horizontal_padding,
                    origin.y() + ((end.row - scroll_top).as_f64() as f32) * cell_height,
                    origin.x() + horizontal_padding + (end.col as f32 + 1.) * cell_width,
                    origin.y() + ((end.row - scroll_top).as_f64() as f32 + 1.) * cell_height
                ),
            ]
        );
    };
    assert_selection_bounds(5.into_lines()); // Without scroll clipping
    assert_selection_bounds(10.into_lines()); // Without scroll clipping (but on the cusp of clipping)
    assert_selection_bounds(80.into_lines()); // With scroll clipping
}

struct ForegroundImageView {
    use_ligatures: bool,
}

impl Entity for ForegroundImageView {
    type Event = ();
}

impl TypedActionView for ForegroundImageView {
    type Action = ();
}

impl View for ForegroundImageView {
    fn ui_name() -> &'static str {
        "foreground_image_test_view"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let metadata = ITermImageMetadata {
            id: 7,
            image_size: vec2f(2., 2.),
            ..Default::default()
        };
        let mut grid = GridHandler::new_for_test(4, 4);
        grid.handle_completed_iterm_image(ITermImage {
            metadata: metadata.clone(),
            data: Vec::new(),
        });
        ForegroundImageElement {
            rect: Rect::new(),
            grid,
            image_metadata: HashMap::from([(7, StoredImageMetadata::ITerm(metadata))]),
            use_ligatures: self.use_ligatures,
        }
        .finish()
    }
}

struct ForegroundImageElement {
    rect: Rect,
    grid: GridHandler,
    image_metadata: HashMap<u32, StoredImageMetadata>,
    use_ligatures: bool,
}

impl Element for ForegroundImageElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.rect.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.rect.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.rect.paint(origin, ctx, app);
        let theme = Appearance::as_ref(app).theme();
        grid_renderer::render_grid(
            &self.grid,
            0,
            4,
            &color::List::from(&color::Colors::from(theme.clone())),
            &color::OverrideList::empty(),
            theme,
            Properties::default(),
            FamilyId(0),
            1.,
            1.,
            vec2f(1., 1.),
            Pixels::zero(),
            origin,
            &mut grid_renderer::CellGlyphCache::default(),
            255,
            None,
            None,
            None::<Empty<&RangeInclusive<Point>>>,
            None,
            EnforceMinimumContrast::Never,
            ObfuscateSecrets::No,
            None,
            self.use_ligatures,
            None,
            RespectDisplayedOutput::No,
            &self.image_metadata,
            None,
            false,
            ctx,
            app,
        );
    }

    fn size(&self) -> Option<Vector2F> {
        self.rect.size()
    }

    fn origin(&self) -> Option<ScreenPoint> {
        self.rect.origin()
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.rect.dispatch_event(event, ctx, app)
    }
}

fn assert_foreground_image_does_not_cover_terminal(use_ligatures: bool) {
    let _images = FeatureFlag::ITermImages.override_enabled(true);
    App::test((), move |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.update(|ctx| {
            AssetCache::handle(ctx).update(ctx, |cache, ctx| {
                cache.insert_raw_asset_bytes::<ImageType>(
                    "7".into(),
                    b"warp-img:rgba:2:2:\xff\x00\x00\xff\xff\x00\x00\xff\xff\x00\x00\xff\xff\x00\x00\xff",
                    ctx,
                );
            });
        });
        let (window_id, _view) = app.add_window(platform::WindowStyle::NotStealFocus, |_| {
            ForegroundImageView { use_ligatures }
        });
        app.update(|ctx| {
            let mut presenter = Presenter::new(window_id);
            let mut updated = EntityIdSet::default();
            updated.insert(ctx.root_view_id(window_id).unwrap());
            presenter.invalidate(
                WindowInvalidation {
                    updated,
                    ..Default::default()
                },
                ctx,
            );
            let scene = presenter.build_scene(vec2f(4., 4.), 1., None, ctx);
            let image_layer = scene.layers().nth(1).expect("前景图片必须绘制在高层");
            assert_eq!(image_layer.images.len(), 1);
            assert_eq!(
                image_layer.images[0].bounds,
                RectF::new(vec2f(0., 0.), vec2f(2., 2.))
            );
            assert!(!scene.is_covered(ScreenPoint::new(1., 1., ZIndex::Normal(0))));
        });
    });
}

#[test]
fn foreground_image_does_not_cover_terminal_without_ligatures() {
    assert_foreground_image_does_not_cover_terminal(false);
}

#[test]
fn foreground_image_does_not_cover_terminal_with_ligatures() {
    assert_foreground_image_does_not_cover_terminal(true);
}
