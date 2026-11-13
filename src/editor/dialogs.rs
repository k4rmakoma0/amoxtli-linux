//! The dialogs some tools ask with before inserting: a link and a table.

use crate::i18n::{fill, tr};
use crate::markdown::format::{normalized_link, TableDimensions};
use adw::prelude::*;

/// Asks for a link's text and address. Returns the text and the normalized address.
pub async fn ask_link(parent: &impl IsA<gtk::Widget>, selected: &str) -> Option<(String, String)> {
    let text = adw::EntryRow::builder().title(tr("Text to display")).text(selected).build();
    let address = adw::EntryRow::builder()
        .title(tr("Address"))
        .input_purpose(gtk::InputPurpose::Url)
        .activates_default(true)
        .build();
    let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    list.append(&text);
    list.append(&address);

    let dialog = adw::AlertDialog::new(Some(&tr("Insert Link")), None);
    dialog.add_responses(&[("cancel", &tr("Cancel")), ("insert", &tr("Insert"))]);
    dialog.set_response_appearance("insert", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("insert"));
    dialog.set_close_response("cancel");
    dialog.set_response_enabled("insert", false);
    dialog.set_extra_child(Some(&list));
    address.connect_changed(glib::clone!(
        #[weak]
        dialog,
        move |row| dialog.set_response_enabled("insert", normalized_link(&row.text()).is_some())
    ));
    dialog.set_focus(Some(if selected.trim().is_empty() { &text } else { &address }));

    if dialog.choose_future(Some(parent)).await != "insert" {
        return None;
    }
    let destination = normalized_link(&address.text())?;
    Some((text.text().to_string(), destination))
}

/// Asks for a table's size. The first row is the header.
pub async fn ask_table(parent: &impl IsA<gtk::Widget>) -> Option<TableDimensions> {
    let spin = |title: String, max: f64| {
        let row = adw::SpinRow::builder()
            .title(title)
            .adjustment(&gtk::Adjustment::new(2.0, 1.0, max, 1.0, 5.0, 0.0))
            .numeric(true)
            .build();
        row
    };
    let columns = spin(tr("Columns"), *TableDimensions::COLUMNS.end() as f64);
    let rows = spin(tr("Rows"), *TableDimensions::ROWS.end() as f64);
    let list = gtk::ListBox::builder().css_classes(["boxed-list"]).selection_mode(gtk::SelectionMode::None).build();
    list.append(&columns);
    list.append(&rows);

    let body = format!("{}\n{}", tr("The first row is the header."), tr("Use 1–20 columns and 1–100 rows."));
    let dialog = adw::AlertDialog::new(Some(&tr("Insert Table")), Some(&body));
    dialog.add_responses(&[("cancel", &tr("Cancel")), ("insert", &tr("Insert"))]);
    dialog.set_response_appearance("insert", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("insert"));
    dialog.set_close_response("cancel");
    dialog.set_extra_child(Some(&list));

    if dialog.choose_future(Some(parent)).await != "insert" {
        return None;
    }
    TableDimensions::new(columns.value() as usize, rows.value() as usize, |n| {
        fill(&tr("Column {n}"), &[("n", &n.to_string())])
    })
}

use gtk::glib;
