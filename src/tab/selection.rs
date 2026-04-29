// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use cosmic::{
    Element, cosmic_theme,
    iced::{Alignment, ContentFit, Length, padding, widget::stack},
    theme, widget,
    widget::menu::action::MenuAction,
};
use mime_guess::Mime;
use trash::TrashItemSize;

use crate::{app::Action, config::IconSizes, fl, menu, mime_app};

use super::{
    DirSize, Item, ItemMetadata, Location, MODE_NAMES, MODE_SHIFT_GROUP, MODE_SHIFT_OTHER,
    MODE_SHIFT_USER, Message, THUMBNAIL_SIZE, Tab, format_size, get_mode_part,
};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum SelectionMenuAction {
    Open,
    Duplicate,
    MoveTo,
    RestoreFromTrash,
}

impl SelectionMenuAction {
    pub(crate) fn app_action(self) -> Action {
        match self {
            Self::Open => Action::Open,
            Self::Duplicate => Action::Duplicate,
            Self::MoveTo => Action::MoveTo,
            Self::RestoreFromTrash => Action::RestoreFromTrash,
        }
    }

    pub(crate) fn label(self) -> String {
        match self {
            Self::Open => fl!("open"),
            Self::Duplicate => fl!("duplicate"),
            Self::MoveTo => fl!("move-to"),
            Self::RestoreFromTrash => fl!("restore-from-trash"),
        }
    }
}

impl MenuAction for SelectionMenuAction {
    type Message = Message;

    fn message(&self) -> Self::Message {
        Message::ContextAction(self.app_action())
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SelectionStats {
    selected_items: usize,
    selected_files: usize,
    selected_folders: usize,
    contained_items: u64,
    contains_items_known: bool,
    total_size: u64,
    calculating_dir_size: bool,
    unknown_size: bool,
    dir_size_error: Option<String>,
}

impl SelectionStats {
    pub(crate) fn has_selection(&self) -> bool {
        self.selected_items > 0
    }

    fn can_open(&self) -> bool {
        (self.selected_items > 0 && self.selected_folders == 0)
            || (self.selected_folders == 1 && self.selected_items == 1)
    }

    fn files_label(&self) -> String {
        fl!("file-count", count = self.selected_files)
    }

    fn folders_label(&self) -> String {
        fl!("folder-count", count = self.selected_folders)
    }

    fn contains_label(&self) -> Option<String> {
        if self.selected_folders == 0 || !self.contains_items_known {
            return None;
        }

        Some(fl!("contains-item-count", count = self.contained_items))
    }

    fn breakdown_text(&self) -> String {
        let mut parts = Vec::with_capacity(2);
        if self.selected_files > 0 {
            parts.push(self.files_label());
        }
        if self.selected_folders > 0 {
            parts.push(self.folders_label());
        }

        let mut text = parts.join(", ");
        if let Some(contains) = self.contains_label() {
            text.push_str(&format!(" ({contains})"));
        }
        text
    }

    pub(crate) fn size_text(&self) -> String {
        if self.calculating_dir_size || self.unknown_size {
            fl!("calculating")
        } else if let Some(error) = &self.dir_size_error {
            error.clone()
        } else {
            format_size(self.total_size)
        }
    }

    pub(crate) fn footer_summary(&self) -> String {
        fl!(
            "selection-footer-summary",
            selection = self.breakdown_text(),
            size = self.size_text()
        )
    }
}

pub(crate) fn selection_menu_actions(
    stats: &SelectionStats,
    in_trash: bool,
    include_move_to: bool,
) -> Vec<SelectionMenuAction> {
    let mut actions = Vec::new();

    if in_trash {
        actions.push(SelectionMenuAction::RestoreFromTrash);
        if include_move_to {
            actions.push(SelectionMenuAction::MoveTo);
        }
    } else {
        if stats.can_open() {
            actions.push(SelectionMenuAction::Open);
        }
        actions.push(SelectionMenuAction::Duplicate);
        if include_move_to {
            actions.push(SelectionMenuAction::MoveTo);
        }
    }

    actions
}

impl Item {
    pub(crate) fn dir_size_path(&self) -> Option<PathBuf> {
        match &self.metadata {
            ItemMetadata::Trash { metadata, entry }
                if matches!(metadata.size, TrashItemSize::Entries(_)) =>
            {
                let info_file = Path::new(&entry.id);
                let trash_folder = info_file.parent()?.parent()?;
                let name_in_trash = info_file.file_stem()?;
                Some(trash_folder.join("files").join(name_in_trash))
            }
            _ => self.path_opt().cloned(),
        }
    }
}

impl Tab {
    pub(crate) fn selection_stats(&self) -> Option<SelectionStats> {
        let items = self.items_opt()?;
        let mut stats = SelectionStats {
            contains_items_known: true,
            ..SelectionStats::default()
        };

        for item in items {
            if !item.selected {
                continue;
            }

            stats.selected_items += 1;

            if item.metadata.is_dir() {
                stats.selected_folders += 1;
                match &item.metadata {
                    ItemMetadata::Path { children_opt, .. } => {
                        if let Some(children) = children_opt {
                            stats.contained_items =
                                stats.contained_items.saturating_add(*children as u64);
                        } else {
                            stats.contains_items_known = false;
                        }
                    }
                    ItemMetadata::Trash { metadata, .. } => match metadata.size {
                        TrashItemSize::Entries(entries) => {
                            stats.contained_items =
                                stats.contained_items.saturating_add(entries as u64);
                        }
                        TrashItemSize::Bytes(_) => {
                            stats.contains_items_known = false;
                        }
                    },
                    ItemMetadata::SimpleDir { entries } => {
                        stats.contained_items = stats.contained_items.saturating_add(*entries);
                    }
                    ItemMetadata::SimpleFile { .. } => {}
                    #[cfg(feature = "gvfs")]
                    ItemMetadata::GvfsPath { children_opt, .. } => {
                        if let Some(children) = children_opt {
                            stats.contained_items =
                                stats.contained_items.saturating_add(*children as u64);
                        } else {
                            stats.contains_items_known = false;
                        }
                    }
                }

                match &item.dir_size {
                    DirSize::Calculating(_) => {
                        stats.calculating_dir_size = true;
                    }
                    DirSize::Directory(size) => {
                        stats.total_size = stats.total_size.saturating_add(*size);
                    }
                    DirSize::NotDirectory => {
                        stats.unknown_size = true;
                    }
                    DirSize::Error(err) => {
                        if stats.dir_size_error.is_none() {
                            stats.dir_size_error = Some(err.clone());
                        }
                    }
                }
            } else {
                stats.selected_files += 1;
                if let Some(size) = item.metadata.file_size() {
                    stats.total_size = stats.total_size.saturating_add(size);
                } else {
                    stats.unknown_size = true;
                }
            }
        }

        Some(stats)
    }

    pub(crate) fn all_selectable_items_selected(&self) -> bool {
        self.items_opt().is_some_and(|items| {
            let mut selectable_items = 0;
            let mut selected_items = 0;

            for item in items {
                if !self.config.show_hidden && item.hidden {
                    continue;
                }

                selectable_items += 1;
                if item.selected {
                    selected_items += 1;
                }
            }

            selectable_items > 0 && selected_items == selectable_items
        })
    }

    pub(crate) fn multi_preview_view<'a>(
        &'a self,
        mime_app_cache_opt: Option<&'a mime_app::MimeAppCache>,
    ) -> Element<'a, Message> {
        let cosmic_theme::Spacing {
            space_xxxs,
            space_xxs,
            space_m,
            ..
        } = theme::active().cosmic().spacing;

        let mut column = widget::column::with_capacity(4).spacing(space_m);

        let handle = widget::icon::from_name("text-x-generic")
            .size(IconSizes::default().grid())
            .handle();

        let icon = widget::icon::icon(handle.clone())
            .content_fit(ContentFit::Contain)
            .size(IconSizes::default().grid());

        let icon_container1 = widget::container(icon.clone()).padding(padding::bottom(10).left(10));
        let icon_container2 =
            widget::container(icon.clone()).padding(padding::top(5).bottom(5).left(5).right(5));
        let icon_container3 = widget::container(icon).padding(padding::top(10).right(10));
        let preview_stack: Element<'a, Message> =
            stack![icon_container1, icon_container2, icon_container3].into();

        column = column.push(
            widget::container(preview_stack)
                .center_x(Length::Fill)
                .max_height(THUMBNAIL_SIZE as f32),
        );

        let selected_items: Vec<&Item> = self.items_opt().map_or(Vec::new(), |items| {
            items.iter().filter(|item| item.selected).collect()
        });
        let selection_stats = self.selection_stats().unwrap_or_default();
        let in_trash = self.location.is_trash();
        let all_selected = self.all_selectable_items_selected();
        let selection_button_label = if all_selected {
            fl!("deselect-all")
        } else {
            fl!("select-all")
        };
        let selection_button_message = if all_selected {
            Message::SelectNone
        } else {
            Message::SelectAll
        };

        let overflow_items = selection_menu_actions(&selection_stats, in_trash, in_trash);

        let mut action_row = widget::row::with_capacity(4)
            .spacing(space_xxs)
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .push(
                widget::button::standard(selection_button_label).on_press(selection_button_message),
            );
        if !in_trash {
            action_row = action_row.push(
                widget::button::standard(fl!("move-to"))
                    .on_press(Message::ContextAction(Action::MoveTo)),
            );
        }
        action_row = action_row
            .push(widget::space::horizontal())
            .push(
                widget::button::icon(widget::icon::from_name("edit-delete-symbolic"))
                    .padding(8)
                    .on_press(Message::ContextAction(Action::Delete)),
            )
            .push(
                widget::menu::MenuBar::new(vec![widget::menu::Tree::with_children(
                    Element::from(
                        widget::button::icon(widget::icon::from_name("view-more-symbolic"))
                            .padding(8)
                            .on_press_maybe(None),
                    ),
                    overflow_items
                        .into_iter()
                        .map(|action| {
                            menu::menu_tree_item(
                                action.label(),
                                None,
                                menu::custom_menu_item_button_class(
                                    menu::standard_menu_surface_color,
                                ),
                                action.message(),
                            )
                        })
                        .collect(),
                )])
                .item_height(widget::menu::ItemHeight::Dynamic(40))
                .item_width(widget::menu::ItemWidth::Uniform(220))
                .style(menu::standard_overlay_menu_style as fn(&theme::Theme) -> _),
            );

        column = column.push(action_row);
        column = column.push(
            widget::column::with_children([
                widget::text::body(fl!("items", items = selection_stats.selected_items)).into(),
                widget::text::body(selection_stats.breakdown_text()).into(),
                widget::text::body(fl!("item-size", size = selection_stats.size_text())).into(),
            ])
            .spacing(space_xxxs),
        );

        let mut mime_type_counts: BTreeMap<String, u64> = BTreeMap::new();
        for item in selected_items.iter() {
            *mime_type_counts.entry(item.mime.to_string()).or_insert(0) += 1;
        }
        let mut mime_types: Vec<(String, u64)> = mime_type_counts.into_iter().collect();
        mime_types.sort_by(|(_, v1), (_, v2)| v2.cmp(v1));

        let limit = usize::min(10, mime_types.len());
        let mut mime_type_strings: Vec<String> = mime_types[..limit]
            .iter()
            .map(|(mime, count)| format!("{} ({})", mime, count))
            .collect();
        if mime_types.len() > limit {
            mime_type_strings.push("...".to_string());
        }

        let details = widget::column::with_children([widget::text::body(fl!(
            "type",
            mime = mime_type_strings.join(", ")
        ))
        .into()])
        .spacing(space_xxxs);
        column = column.push(details);

        if in_trash {
            return column.into();
        }

        let selected_items: Vec<&Item> = selected_items
            .into_iter()
            .filter(|item| {
                item.location_opt
                    .as_ref()
                    .and_then(Location::path_opt)
                    .is_some()
            })
            .collect();

        let mut user_name: BTreeSet<String> = BTreeSet::new();
        let mut mode_user: BTreeSet<u32> = BTreeSet::new();
        let mut group_name: BTreeSet<String> = BTreeSet::new();
        let mut mode_group: BTreeSet<u32> = BTreeSet::new();
        let mut mode_other: BTreeSet<u32> = BTreeSet::new();

        for item in selected_items.iter() {
            if let Some(metadata) = item.file_metadata() {
                #[cfg(unix)]
                {
                    let mode = metadata.mode();
                    user_name.insert(
                        uzers::get_user_by_uid(metadata.uid())
                            .and_then(|user| user.name().to_str().map(ToOwned::to_owned))
                            .unwrap_or_default(),
                    );
                    mode_user.insert(get_mode_part(mode, MODE_SHIFT_USER));
                    group_name.insert(
                        uzers::get_group_by_gid(metadata.gid())
                            .and_then(|group| group.name().to_str().map(ToOwned::to_owned))
                            .unwrap_or_default(),
                    );
                    mode_group.insert(get_mode_part(mode, MODE_SHIFT_GROUP));
                    mode_other.insert(get_mode_part(mode, MODE_SHIFT_OTHER));
                }
            }
        }

        let mut settings = Vec::new();
        if mime_types.len() == 1 {
            if let Some(mime) = mime_types
                .get(0)
                .and_then(|(mime, _)| mime.parse::<Mime>().ok())
            {
                if let Some(mime_app_cache) = mime_app_cache_opt {
                    let mime_apps = mime_app_cache.get(&mime);
                    if !mime_apps.is_empty() {
                        let mime_closure = mime.clone();
                        settings.push(
                            widget::settings::item::builder(fl!("open-with")).control(
                                Element::from(
                                    widget::dropdown(
                                        mime_apps,
                                        mime_apps.iter().position(|x| x.is_default),
                                        move |index| (index, mime_closure.clone()),
                                    )
                                    .icons(Cow::Borrowed(mime_app_cache.icons(&mime))),
                                )
                                .map(|(index, mime)| {
                                    let mime_app = &mime_apps[index];
                                    Message::SetOpenWith(mime, mime_app.id.clone())
                                }),
                            ),
                        );
                    }
                }
            }
        }

        #[cfg(unix)]
        {
            fn selected_mode_part(mut modes: BTreeSet<u32>) -> Option<usize> {
                match (modes.pop_first(), modes.pop_first()) {
                    (Some(mode), None) => Some(mode.try_into().unwrap()),
                    _ => None,
                }
            }

            fn join_set(set: BTreeSet<String>) -> String {
                let limit = 5;
                let mut title = set.into_iter().collect::<Vec<String>>();
                if title.len() > limit {
                    title.truncate(limit);
                    title.push("...".to_string());
                }
                title.join(", ")
            }

            let mode_part_user = selected_mode_part(mode_user);
            settings.push(
                widget::settings::item::builder(join_set(user_name))
                    .description(fl!("owner"))
                    .control(
                        widget::dropdown(
                            Cow::Borrowed(MODE_NAMES.as_slice()),
                            mode_part_user,
                            move |selected| {
                                Message::ShiftPermissions(
                                    None,
                                    MODE_SHIFT_USER,
                                    selected.try_into().unwrap(),
                                )
                            },
                        )
                        .placeholder(fl!("mixed")),
                    ),
            );

            let mode_part_group = selected_mode_part(mode_group);
            settings.push(
                widget::settings::item::builder(join_set(group_name))
                    .description(fl!("group"))
                    .control(
                        widget::dropdown(
                            Cow::Borrowed(MODE_NAMES.as_slice()),
                            mode_part_group,
                            move |selected| {
                                Message::ShiftPermissions(
                                    None,
                                    MODE_SHIFT_GROUP,
                                    selected.try_into().unwrap(),
                                )
                            },
                        )
                        .placeholder(fl!("mixed")),
                    ),
            );

            let mode_part_other = selected_mode_part(mode_other);
            settings.push(
                widget::settings::item::builder(fl!("other")).control(
                    widget::dropdown(
                        Cow::Borrowed(MODE_NAMES.as_slice()),
                        mode_part_other,
                        move |selected| {
                            Message::ShiftPermissions(
                                None,
                                MODE_SHIFT_OTHER,
                                selected.try_into().unwrap(),
                            )
                        },
                    )
                    .placeholder(fl!("mixed")),
                ),
            );
        }

        if !settings.is_empty() {
            let mut section = widget::settings::section();
            section = section.extend(settings);
            column = column.push(section);
        }

        column.into()
    }
}
