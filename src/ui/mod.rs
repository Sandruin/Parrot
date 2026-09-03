pub mod style;

use std::path::PathBuf;

use crate::engine::EngineHandle;
use crate::model::{ActionId, AppSettings, EngineEvent, Macro};

/// What the app is currently doing; drives which buttons are enabled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Idle,
    Recording,
    Playing,
}

pub struct App {
    pub doc: Macro,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub selected: Option<ActionId>,
    pub mode: Mode,
    pub settings: AppSettings,
    pub engine: EngineHandle,
    pub status: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, engine: EngineHandle, settings: AppSettings) -> Self {
        style::apply(&cc.egui_ctx);
        Self {
            doc: Macro::default(),
            path: None,
            dirty: false,
            selected: None,
            mode: Mode::Idle,
            settings,
            engine,
            status: "Ready".into(),
        }
    }

    fn handle_event(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::Error(msg) => self.status = msg,
            other => self.status = format!("{other:?}"),
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for event in self.engine.drain() {
            self.handle_event(event);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("Macro Recorder");
            });
            ui.add_space(4.0);
        });

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("{} actions", self.doc.items.len()));
                ui.separator();
                ui.label(&self.status);
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.centered_and_justified(|ui| {
                ui.weak("No actions yet. Record or add one from the toolbar.");
            });
        });
    }
}
