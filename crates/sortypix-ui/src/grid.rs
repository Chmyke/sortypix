use crate::{
    commands::{UiCommand, click},
    gallery::{ACCENT, Gallery},
    textures,
};
use eframe::egui::{self, Rect, Sense};
use sortypix_app::worker::Command;
use sortypix_core::Decision;

fn should_record_resume(busy: bool, restore_position: bool, catalog_final: bool) -> bool {
    !busy && !restore_position && catalog_final
}

impl Gallery {
    pub(crate) fn grid(&mut self, ui: &mut egui::Ui) {
        self.grid_rect = ui.available_rect_before_wrap();
        let width = geometry(ui);
        self.layout.update(
            width,
            self.settings.thumbnail_size * (2. / 3.),
            self.photos.iter().zip(&self.turns).map(|(photo, turn)| {
                if turn % 2 == 1 {
                    1. / photo.aspect_ratio
                } else {
                    photo.aspect_ratio
                }
            }),
        );
        let scroll = egui::ScrollArea::vertical()
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible);
        let scroll = if let Some(resume) = self.pending_transition.take() {
            if let Some(root) = &self.root
                && let Some((index, offset)) =
                    transition_offset(&resume, root, &self.photos, &self.layout)
            {
                self.selected = index;
                scroll.vertical_scroll_offset(offset)
            } else {
                scroll
            }
        } else if self.restore_position && !self.busy {
            let mut offset = 0.;
            if let Some(resume) = &self.settings.resume
                && let Some(root) = &self.root
                && let Some(index) = resume.locate(root, &self.photos)
            {
                self.selected = index;
                if let Some(cell) = self.layout.cells.get(index) {
                    offset = cell.top() + cell.height() * resume.fraction.clamp(0., 1.);
                }
            }
            self.restore_position = false;
            scroll.vertical_scroll_offset(offset)
        } else {
            scroll
        };
        let scroll = if self.grid_scroll_to_focus {
            scroll.vertical_scroll_offset(
                self.layout.cells.get(self.selected).map_or(0., |r| r.top()),
            )
        } else {
            scroll
        };
        let scroll = if let Some(b) = &self.benchmark {
            scroll.vertical_scroll_offset(b.scroll)
        } else {
            scroll
        };
        let output = scroll
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                ui.set_min_size(egui::vec2(width, self.layout.height));
                let origin = ui.min_rect().min.to_vec2();
                let visible = self.layout.visible(viewport.top(), viewport.bottom());
                if should_record_resume(self.busy, self.restore_position, self.catalog_final)
                    && let Some(row) = self.layout.rows.get(visible.start)
                    && let Some(photo) = self.photos.get(row.start)
                    && let Some(root) = &self.root
                {
                    let cell = self.layout.cells[row.start];
                    self.settings.resume = Some(sortypix_app::settings::Resume {
                        folder: root.clone(),
                        photo: photo.display_source(false).to_path_buf(),
                        index: row.start,
                        fraction: ((viewport.top() - cell.top()) / cell.height()).clamp(0., 1.),
                    });
                }
                for row in visible.clone() {
                    for index in self.layout.rows[row].clone() {
                        let photo = self.photos[index].clone();
                        let turns = self.turns[index];
                        let rect = self.layout.cells[index].translate(origin);
                        let response = ui.interact(rect, ui.id().with(photo.id.0), Sense::click());
                        let painter = ui.painter();
                        if index == 2
                            && let Some(d) = &mut self.diagnostic
                        {
                            d.click_target = Some(rect.center());
                            d.rotate_target = crate::icons::left_corner(rect).map(|r| r.center());
                        }
                        painter.rect_filled(rect, 8, crate::theme::CARD);
                        let photo_rect = rect;
                        let source = photo.display_source(false);
                        let edge = textures::thumbnail_edge(
                            photo_rect.size(),
                            ui.ctx().pixels_per_point(),
                        );
                        if let Some(texture) = self
                            .textures
                            .visible(source, edge)
                            .or_else(|| self.textures.cached(source, 800))
                        {
                            let size = textures::fit(texture.size_vec2(), photo_rect.size(), turns);
                            textures::paint(
                                painter,
                                Rect::from_center_size(photo_rect.center(), size),
                                &texture,
                                turns,
                            );
                        } else {
                            painter.text(
                                photo_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                if self.textures.error(source, edge).is_some() {
                                    "Preview unavailable"
                                } else {
                                    "Loading…"
                                },
                                egui::FontId::proportional(13.),
                                crate::theme::DIM,
                            );
                        }
                        if index == self.selected {
                            ui.painter().rect_stroke(
                                rect,
                                8,
                                egui::Stroke::new(2., ACCENT),
                                egui::StrokeKind::Inside,
                            );
                        }
                        let inspect =
                            inspect_overlay(ui, &response, rect, photo.id.0).is_some_and(|r| {
                                r.on_hover_text(self.shortcut_tip(crate::shortcuts::Action::Expand))
                                    .clicked()
                            });
                        let rotate = rotate_overlay(ui, &response, rect, photo.id.0)
                            .is_some_and(|r| r.on_hover_text(self.rotate_tip()).clicked());
                        if rotate && !self.busy && !self.shortcut_editor.open {
                            self.selected = index;
                            let right = !ui.input(|i| i.modifiers.shift);
                            self.send(Command::Rotate(photo.id, right));
                        } else if inspect && !self.shortcut_editor.open {
                            self.selected = index;
                            self.viewer = true;
                            self.zoom = false;
                            self.pan = egui::Vec2::ZERO;
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
                        } else if !self.busy
                            && !self.shortcut_editor.open
                            && (response.clicked()
                                || response.secondary_clicked()
                                || response.middle_clicked())
                        {
                            self.selected = index;
                            let control = ui.input(|i| i.modifiers.ctrl);
                            match click(
                                photo.id,
                                response.secondary_clicked(),
                                self.settings.inverted,
                                control,
                                response.middle_clicked(),
                                response.dragged(),
                            ) {
                                Some(UiCommand::Decide(id, decision)) => {
                                    self.decide(vec![id], decision)
                                }
                                Some(UiCommand::RejectThrough(_)) => self.decide(
                                    self.photos[..=index].iter().map(|p| p.id).collect(),
                                    Decision::Reject,
                                ),
                                None => {}
                            }
                        }
                        if self.grid_scroll_to_focus && index == self.selected {
                            response.scroll_to_me(Some(egui::Align::Center));
                            self.grid_scroll_to_focus = false;
                        }
                    }
                }
                self.scroll_down =
                    scroll_direction(self.scroll_offset, viewport.top(), self.scroll_down);
                let rows = prefetch_rows(visible, self.layout.rows.len(), self.scroll_down);
                for row in rows {
                    for index in self.layout.rows[row].clone() {
                        let edge = textures::thumbnail_edge(
                            self.layout.cells[index].size(),
                            ui.ctx().pixels_per_point(),
                        );
                        self.textures
                            .prefetch(self.photos[index].display_source(false), edge);
                    }
                }
            });
        self.scroll_offset = output.state.offset.y;
    }
}

fn transition_offset(
    resume: &sortypix_app::settings::Resume,
    root: &std::path::Path,
    photos: &[sortypix_app::catalog::PhysicalPhoto],
    layout: &crate::layout::Layout,
) -> Option<(usize, f32)> {
    let index = resume.locate(root, photos)?;
    let cell = layout.cells.get(index)?;
    Some((
        index,
        cell.top() + cell.height() * resume.fraction.clamp(0., 1.),
    ))
}

fn scroll_direction(previous: f32, current: f32, down: bool) -> bool {
    if (current - previous).abs() < 0.1 {
        down
    } else {
        current > previous
    }
}

fn prefetch_rows(
    visible: std::ops::Range<usize>,
    count: usize,
    down: bool,
) -> std::ops::Range<usize> {
    if down {
        visible.end..(visible.end + 2).min(count)
    } else {
        visible.start.saturating_sub(2)..visible.start
    }
}

pub(crate) fn resize_input(
    input: &mut egui::RawInput,
    previous: Option<egui::Pos2>,
    rect: Rect,
    size: &mut f32,
) {
    let mut pointer = previous;
    input.events.retain(|event| {
        match event {
            egui::Event::PointerMoved(pos) => pointer = Some(*pos),
            egui::Event::PointerGone => pointer = None,
            egui::Event::MouseWheel {
                unit,
                delta,
                modifiers,
                ..
            } if modifiers.ctrl && pointer.is_some_and(|p| rect.contains(p)) => {
                let scale = match unit {
                    egui::MouseWheelUnit::Point => 1.,
                    egui::MouseWheelUnit::Line => 40.,
                    egui::MouseWheelUnit::Page => 200.,
                };
                *size = (*size * (delta.y * scale * 0.002).exp()).clamp(
                    sortypix_app::settings::MIN_THUMBNAIL_SIZE,
                    sortypix_app::settings::MAX_THUMBNAIL_SIZE,
                );
                return false;
            }
            _ => {}
        }
        true
    });
}

fn geometry(ui: &mut egui::Ui) -> f32 {
    ui.spacing_mut().item_spacing = egui::vec2(crate::layout::GAP, crate::layout::GAP);
    ui.spacing_mut().scroll.floating = false;
    ui.spacing_mut().scroll.bar_width = 8.;
    ui.spacing_mut().scroll.bar_inner_margin = 6.;
    ui.spacing_mut().scroll.bar_outer_margin = 0.;
    ui.visuals_mut().widgets.inactive.fg_stroke.color = crate::theme::DIM;
    ui.visuals_mut().widgets.hovered.fg_stroke.color = crate::theme::BLUE;
    (ui.available_width() - ui.spacing().scroll.allocated_width()).max(1.)
}

fn inspect_overlay(
    ui: &mut egui::Ui,
    photo: &egui::Response,
    photo_rect: Rect,
    id: u64,
) -> Option<egui::Response> {
    let button = crate::icons::corner(photo_rect);
    corner_control(
        ui,
        photo,
        button,
        ("expand", id),
        crate::icons::Icon::Expand,
    )
}

/// Rotation sits on the photo it turns, mirroring the expansion control.
fn rotate_overlay(
    ui: &mut egui::Ui,
    photo: &egui::Response,
    photo_rect: Rect,
    id: u64,
) -> Option<egui::Response> {
    let button = crate::icons::left_corner(photo_rect)?;
    corner_control(
        ui,
        photo,
        button,
        ("rotate", id),
        crate::icons::Icon::Rotate,
    )
}

fn corner_control(
    ui: &mut egui::Ui,
    photo: &egui::Response,
    button: Rect,
    salt: (&str, u64),
    icon: crate::icons::Icon,
) -> Option<egui::Response> {
    let response = ui.interact(button, ui.id().with(salt), Sense::click());
    // The parent loses `hovered` to this button. Physical containment stays stable.
    if !ui.rect_contains_pointer(photo.rect) && !response.has_focus() {
        return None;
    }
    let painter = ui
        .painter()
        .with_clip_rect(button.intersect(ui.clip_rect()));
    painter.rect_filled(button, 7, egui::Color32::from_black_alpha(220));
    let color = if response.hovered() {
        crate::theme::BLUE
    } else {
        crate::theme::TEXT
    };
    crate::icons::paint(&painter, button, icon, color);
    Some(response.on_hover_cursor(egui::CursorIcon::PointingHand))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn control_wheel_resizes_photos_without_leaking_into_scroll_or_ui_zoom() {
        let rect = Rect::from_min_max(egui::pos2(0., 40.), egui::pos2(800., 600.));
        let wheel = |ctrl| egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0., 1.),
            phase: egui::TouchPhase::Move,
            modifiers: if ctrl {
                egui::Modifiers::CTRL
            } else {
                egui::Modifiers::NONE
            },
        };
        let mut size = 190.;
        let mut input = egui::RawInput {
            events: vec![egui::Event::PointerMoved(rect.center()), wheel(true)],
            ..Default::default()
        };
        resize_input(&mut input, None, rect, &mut size);
        assert!(size > 190.);
        assert_eq!(input.events.len(), 1);
        let previous = size;
        input.events = vec![wheel(false)];
        resize_input(&mut input, Some(rect.center()), rect, &mut size);
        assert_eq!(size, previous);
        assert_eq!(input.events.len(), 1);
        input.events = vec![wheel(true)];
        resize_input(&mut input, Some(egui::pos2(10., 10.)), rect, &mut size);
        assert_eq!(size, previous);
        assert_eq!(input.events.len(), 1);
    }
    #[test]
    fn prefetch_follows_direction_and_stops_at_folder_edges() {
        assert!(!scroll_direction(500., 400., true));
        assert!(!scroll_direction(400., 400., false));
        assert!(scroll_direction(400., 500., false));
        assert_eq!(prefetch_rows(4..8, 20, true), 8..10);
        assert_eq!(prefetch_rows(4..8, 20, false), 2..4);
        assert_eq!(prefetch_rows(0..3, 20, false), 0..0);
        assert_eq!(prefetch_rows(17..20, 20, true), 20..20);
    }
    #[test]
    fn gallery_width_reserves_the_scrollbar_gutter() {
        egui::__run_test_ui(|ui| {
            let width = geometry(ui);
            let scroll = egui::ScrollArea::vertical()
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                .show_viewport(ui, |ui, _| {
                    ui.set_min_size(egui::vec2(width, 2000.));
                });
            assert!((width - scroll.inner_rect.width()).abs() < 0.1);
        });
    }
    #[test]
    fn expansion_control_stays_visible_while_pointer_is_over_it() {
        let ctx = egui::Context::default();
        let mut opened = 0;
        let mut sorted = 0;
        for frame in 0..12 {
            let mut events = vec![egui::Event::PointerMoved(egui::pos2(170., 155.))];
            if frame == 6 || frame == 7 {
                events.push(egui::Event::PointerButton {
                    pos: egui::pos2(170., 155.),
                    button: egui::PointerButton::Primary,
                    pressed: frame == 6,
                    modifiers: egui::Modifiers::default(),
                });
            }
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 600.),
                )),
                events,
                ..Default::default()
            };
            let output = ctx.run_ui(input, |ui| {
                let (cell, response) =
                    ui.allocate_exact_size(egui::vec2(190., 175.), Sense::click());
                let icon = inspect_overlay(ui, &response, cell, 1);
                let shown = icon.is_some();
                opened += usize::from(icon.is_some_and(|r| r.clicked()));
                sorted += usize::from(response.clicked());
                if frame > 2 {
                    assert!(shown, "Expansion control disappeared on frame {frame}");
                }
            });
            output.drop_without_applying_deltas();
        }
        assert_eq!(opened, 1, "One icon click opens exactly one photo");
        assert_eq!(sorted, 0, "The expansion click must not sort the photo");
    }
    #[test]
    fn rotation_control_turns_the_photo_without_sorting_it() {
        let ctx = egui::Context::default();
        let target = egui::pos2(20., 155.);
        let mut rotated = 0;
        let mut sorted = 0;
        for frame in 0..8 {
            let mut events = vec![egui::Event::PointerMoved(target)];
            if frame == 4 || frame == 5 {
                events.push(egui::Event::PointerButton {
                    pos: target,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 4,
                    modifiers: egui::Modifiers::default(),
                });
            }
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 600.),
                )),
                events,
                ..Default::default()
            };
            let output = ctx.run_ui(input, |ui| {
                let (cell, response) =
                    ui.allocate_exact_size(egui::vec2(190., 175.), Sense::click());
                let expand = inspect_overlay(ui, &response, cell, 1);
                let rotate = rotate_overlay(ui, &response, cell, 1);
                rotated += usize::from(rotate.as_ref().is_some_and(|r| r.clicked()));
                sorted += usize::from(response.clicked());
                if frame > 1 {
                    let (rotate, expand) = (rotate.unwrap().rect, expand.unwrap().rect);
                    assert!(rotate.left() < cell.center().x && expand.left() > cell.center().x);
                    assert!(!rotate.intersects(expand));
                }
            });
            output.drop_without_applying_deltas();
        }
        assert_eq!(rotated, 1, "One click on the photo control rotates once");
        assert_eq!(sorted, 0, "Rotating must never keep or reject the photo");
        let narrow = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(60., 100.));
        assert!(crate::icons::left_corner(narrow).is_none());
    }
    #[test]
    fn final_catalog_order_restores_the_same_photo_and_fraction() {
        use sortypix_app::{catalog::PhysicalPhoto, settings::Resume};
        use sortypix_core::PhotoId;
        use std::path::PathBuf;
        let root = PathBuf::from("C:/photos");
        let photo = |id, name: &str, ratio| PhysicalPhoto {
            id: PhotoId(id),
            members: vec![root.join(name)],
            aspect_ratio: ratio,
        };
        let photos = vec![
            photo(2, "c.jpg", 0.75),
            photo(0, "a.jpg", 1.5),
            photo(1, "b.jpg", 2.0),
        ];
        let mut layout = crate::layout::Layout::default();
        layout.update(300., 120., photos.iter().map(|photo| photo.aspect_ratio));
        let resume = Resume {
            folder: root.clone(),
            photo: root.join("b.jpg"),
            index: 1,
            fraction: 0.4,
        };

        let (index, offset) = transition_offset(&resume, &root, &photos, &layout).unwrap();

        assert_eq!(index, 2);
        assert!((offset - (layout.cells[2].top() + layout.cells[2].height() * 0.4)).abs() < 0.01);
    }
    #[test]
    fn provisional_catalog_does_not_replace_the_persisted_position() {
        assert!(!should_record_resume(false, false, false));
        assert!(should_record_resume(false, false, true));
    }
    #[test]
    fn hover_overlay_preserves_next_cell_position() {
        egui::__run_test_ui(|ui| {
            ui.horizontal(|ui| {
                let (cell, response) =
                    ui.allocate_exact_size(egui::vec2(190., 175.), Sense::hover());
                let before = ui.cursor();
                inspect_overlay(ui, &response, cell, 1);
                assert_eq!(
                    ui.cursor(),
                    before,
                    "Hover must never advance the row layout"
                );
            });
        });
    }
}
