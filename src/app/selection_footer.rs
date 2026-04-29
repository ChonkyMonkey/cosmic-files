// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

use cosmic::{
    Element,
    app::Task,
    cosmic_theme,
    iced::widget::stack,
    iced::{self, Alignment, Length},
    theme, widget,
    widget::{icon, menu::key_bind::KeyBind, segmented_button::Entity},
};
use std::{collections::BTreeMap, path::PathBuf};
use trash::TrashItem;

use crate::{
    app::{Action, App, ContextPage, FileDialogContext, Message, PreviewKind},
    fl, menu,
    operation::Operation,
    tab,
    tab::{ItemMetadata, Tab},
};

const SELECTION_FOOTER_DIVIDER_TOP_PADDING: u16 = 2;
const SELECTION_FOOTER_VERTICAL_PADDING: u16 = 2;
const FLOATING_FOOTER_SCROLL_INSET: u16 = 56;
const COMPACT_SELECTION_FOOTER_SCROLL_INSET: u16 = 96;
const COMPACT_SELECTION_FOOTER_WIDTH: f32 = 720.0;
const COMPACT_SELECTION_FOOTER_WIDTH_WITH_NAV: f32 = 600.0;
const NAV_BAR_RESERVED_WIDTH: f32 = 288.0;

impl App {
    pub(super) fn selected_trash_items(&self, entity_opt: Option<Entity>) -> Vec<TrashItem> {
        let entity = entity_opt.unwrap_or_else(|| self.tab_model.active());
        self.tab_model
            .data::<Tab>(entity)
            .and_then(Tab::items_opt)
            .into_iter()
            .flatten()
            .filter_map(|item| {
                if item.selected {
                    match &item.metadata {
                        ItemMetadata::Trash { entry, .. } => Some(entry.clone()),
                        _ => None,
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    pub(super) fn duplicate_selected(&mut self, entity_opt: Option<Entity>) -> Task<Message> {
        let mut grouped: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
        for path in self.selected_paths(entity_opt) {
            let Some(parent) = path.parent() else {
                continue;
            };
            grouped
                .entry(parent.to_path_buf())
                .or_default()
                .push(path.to_path_buf());
        }

        Task::batch(
            grouped
                .into_iter()
                .map(|(to, paths)| self.operation(Operation::Copy { paths, to })),
        )
    }

    pub(super) fn move_from_trash(&mut self, items: Vec<TrashItem>) -> Task<Message> {
        let paths: Box<[_]> = items.iter().map(|item| item.original_path()).collect();
        self.destination_selection_dialog(
            &paths,
            FileDialogContext::TrashItems(items),
            Message::MoveToResult,
            fl!("move-to-title"),
            fl!("move-to-button-label"),
        )
    }

    fn floating_footer_surface_color(theme: &theme::Theme) -> iced::Color {
        theme.cosmic().primary.base.into()
    }

    pub(super) fn active_tab_has_selection(&self) -> bool {
        self.tab_model
            .active_data::<Tab>()
            .and_then(Tab::items_opt)
            .is_some_and(|items| items.iter().any(|item| item.selected))
    }

    fn selection_footer_stats(&self) -> Option<(tab::SelectionStats, bool, bool)> {
        let tab = self.tab_model.active_data::<Tab>()?;
        let stats = tab.selection_stats()?;
        stats.has_selection().then_some((
            stats,
            tab.location.is_trash(),
            tab.all_selectable_items_selected(),
        ))
    }

    fn uses_compact_selection_footer(&self) -> bool {
        if self.core.is_condensed() {
            return true;
        }

        self.size.is_some_and(|size| {
            let (nav_bar_width, compact_width) = if self.core.nav_bar_active() {
                (
                    NAV_BAR_RESERVED_WIDTH,
                    COMPACT_SELECTION_FOOTER_WIDTH_WITH_NAV,
                )
            } else {
                (0.0, COMPACT_SELECTION_FOOTER_WIDTH)
            };
            size.width - nav_bar_width < compact_width
        })
    }

    pub(super) fn floating_footer_scroll_inset(&self) -> u16 {
        if self.selection_footer_stats().is_some() && self.uses_compact_selection_footer() {
            COMPACT_SELECTION_FOOTER_SCROLL_INSET
        } else {
            FLOATING_FOOTER_SCROLL_INSET
        }
    }

    fn floating_footer_style(theme: &theme::Theme) -> widget::container::Style {
        let cosmic = theme.cosmic();
        let background = Self::floating_footer_surface_color(theme);

        widget::container::Style {
            icon_color: Some(cosmic.background.component.on.into()),
            text_color: Some(cosmic.background.component.on.into()),
            background: Some(iced::Background::Color(background)),
            border: iced::Border {
                radius: cosmic.corner_radii.radius_s.into(),
                width: 0.0,
                color: iced::Color::TRANSPARENT,
            },
            snap: true,
            ..Default::default()
        }
    }

    fn floating_footer_menu_style(theme: &theme::Theme) -> theme::menu_bar::Appearance {
        menu::overlay_menu_style(theme, Self::floating_footer_surface_color)
    }

    fn floating_footer_menu_item(
        &self,
        label: String,
        action: Action,
    ) -> widget::menu::Tree<Message> {
        let shortcut = self.key_binds.iter().find_map(|(key_bind, bound_action)| {
            (*bound_action == action).then(|| KeyBind::to_string(key_bind))
        });

        menu::menu_tree_item(
            label,
            shortcut,
            menu::custom_menu_item_button_class(Self::floating_footer_surface_color),
            action.message(None),
        )
    }

    fn floating_footer_more_menu_button() -> Element<'static, Message> {
        widget::button::icon(icon::from_name("view-more-symbolic"))
            .padding(8)
            .on_press(Message::None)
            .into()
    }

    fn floating_footer_shell<'a>(&self, content: Element<'a, Message>) -> Element<'a, Message> {
        let cosmic_theme::Spacing {
            space_xxs, space_s, ..
        } = theme::active().cosmic().spacing;
        let footer_edge_padding = space_xxs.saturating_sub(1);

        widget::container(
            widget::container(content)
                .padding([SELECTION_FOOTER_VERTICAL_PADDING + space_xxs, space_xxs])
                .width(Length::Fill)
                .style(Self::floating_footer_style),
        )
        .padding([
            SELECTION_FOOTER_DIVIDER_TOP_PADDING + space_xxs,
            space_s,
            footer_edge_padding,
            footer_edge_padding,
        ])
        .into()
    }

    pub(super) fn trash_footer(&self) -> Option<Element<'_, Message>> {
        let tab = self.tab_model.active_data::<Tab>()?;
        let showing_selection_details = self.core.window.show_context
            && matches!(
                self.context_page,
                ContextPage::Preview(_, PreviewKind::Selected)
            );
        if !tab.location.is_trash()
            || (self.active_tab_has_selection() && !showing_selection_details)
        {
            return None;
        }

        tab.items_opt().filter(|items| !items.is_empty()).map(|_| {
            self.floating_footer_shell(
                widget::row::with_children([
                    widget::space::horizontal().into(),
                    widget::button::standard(fl!("empty-trash"))
                        .on_press(Message::TabMessage(None, tab::Message::EmptyTrash))
                        .into(),
                ])
                .align_y(Alignment::Center)
                .into(),
            )
        })
    }

    pub(super) fn floating_footer(&self) -> Option<Element<'_, Message>> {
        self.selection_footer().or_else(|| self.trash_footer())
    }

    fn selection_footer(&self) -> Option<Element<'_, Message>> {
        let cosmic_theme::Spacing { space_xxs, .. } = theme::active().cosmic().spacing;
        let (stats, in_trash, all_selected) = self.selection_footer_stats()?;
        let compact = self.uses_compact_selection_footer();
        let summary = stats.footer_summary();
        let selection_button_label = if all_selected {
            fl!("deselect-all")
        } else {
            fl!("select-all")
        };
        let selection_button_message = if all_selected {
            tab::Message::SelectNone
        } else {
            tab::Message::SelectAll
        };

        let menu_items = tab::selection_menu_actions(&stats, in_trash, !compact);
        let more_menu = widget::menu::MenuBar::new(vec![widget::menu::Tree::with_children(
            Self::floating_footer_more_menu_button(),
            menu_items
                .into_iter()
                .map(|action| self.floating_footer_menu_item(action.label(), action.app_action()))
                .collect(),
        )])
        .item_height(widget::menu::ItemHeight::Dynamic(40))
        .item_width(widget::menu::ItemWidth::Uniform(220))
        .style(Self::floating_footer_menu_style as fn(&theme::Theme) -> _);

        let select_button = widget::button::standard(selection_button_label)
            .on_press(Message::TabMessage(None, selection_button_message));
        let delete_button = widget::button::icon(icon::from_name("edit-delete-symbolic"))
            .padding(8)
            .on_press(Message::Delete(None));

        let footer_content: Element<_> = if compact {
            let left_actions = widget::row::with_children([
                select_button.into(),
                widget::button::standard(fl!("move-to"))
                    .on_press(Message::MoveTo(None))
                    .into(),
            ])
            .spacing(space_xxs)
            .align_y(Alignment::Center);

            let right_actions =
                widget::row::with_children([delete_button.into(), more_menu.into()])
                    .spacing(space_xxs)
                    .align_y(Alignment::Center);

            widget::column::with_children([
                widget::row::with_children([
                    left_actions.into(),
                    widget::space::horizontal().into(),
                    right_actions.into(),
                ])
                .align_y(Alignment::Center)
                .width(Length::Fill)
                .into(),
                widget::container(widget::text::caption(summary))
                    .width(Length::Fill)
                    .center_x(Length::Fill)
                    .into(),
            ])
            .spacing(space_xxs)
            .into()
        } else {
            let actions = widget::row::with_children([
                select_button.into(),
                delete_button.into(),
                more_menu.into(),
            ])
            .spacing(space_xxs)
            .align_y(Alignment::Center);

            widget::row::with_children([
                widget::text::caption(summary).into(),
                widget::space::horizontal().into(),
                actions.into(),
            ])
            .align_y(Alignment::Center)
            .into()
        };

        Some(self.floating_footer_shell(footer_content))
    }

    pub(super) fn tab_view_with_footer<'a>(
        &'a self,
        tab: &'a Tab,
        entity: Entity,
    ) -> Element<'a, Message> {
        let use_sidebar_selection = self.core.window.show_context
            && matches!(
                self.context_page,
                ContextPage::Preview(_, PreviewKind::Selected)
            );
        let floating_footer = if use_sidebar_selection && tab.location.is_trash() {
            self.trash_footer()
        } else if use_sidebar_selection {
            None
        } else {
            self.floating_footer()
        };
        let footer_scroll_inset = floating_footer
            .as_ref()
            .map(|_| self.floating_footer_scroll_inset())
            .unwrap_or(0);
        let tab_view: Element<_> = tab
            .view(
                &self.key_binds,
                &self.modifiers,
                footer_scroll_inset,
                self.clipboard_has_content(),
                &self.config.context_actions,
            )
            .map(move |message| Message::TabMessage(Some(entity), message));

        if let Some(selection_footer) = floating_footer {
            stack([
                tab_view,
                widget::container(selection_footer)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .align_x(iced::alignment::Horizontal::Center)
                    .align_y(iced::alignment::Vertical::Bottom)
                    .into(),
            ])
            .into()
        } else {
            tab_view
        }
    }
}
