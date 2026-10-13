use eframe::egui;
use std::sync::mpsc::{self, Receiver, Sender};

#[derive(Debug, Clone, Copy)]
pub enum ShellAction {
    Settings,
    Feed,
    Quiet,
    Home,
    Restore,
    Quit,
}

pub struct Shell {
    pub receiver: Receiver<ShellAction>,
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    _tray: tray_icon::TrayIcon,
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    _hotkeys: global_hotkey::GlobalHotKeyManager,
}

impl Shell {
    pub fn new(context: egui::Context) -> Result<Self, Box<dyn std::error::Error>> {
        let (sender, receiver) = mpsc::channel();
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        {
            use global_hotkey::{
                GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
                hotkey::{Code, HotKey, Modifiers},
            };
            use tray_icon::{
                Icon, TrayIconBuilder,
                menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
            };
            let menu = Menu::new();
            let items = [
                ("麻薯 · 控制面板", ShellAction::Settings),
                ("投喂一颗小饼干", ShellAction::Feed),
                ("切换安静模式", ShellAction::Quiet),
                ("回到角落", ShellAction::Home),
                ("立即恢复桌面 / 停止捣蛋", ShellAction::Restore),
                ("退出", ShellAction::Quit),
            ];
            let mut mapping = Vec::new();
            for (index, (label, action)) in items.into_iter().enumerate() {
                if index == 4 {
                    menu.append(&PredefinedMenuItem::separator())?;
                }
                let item = MenuItem::new(label, true, None);
                mapping.push((item.id().clone(), action));
                menu.append(&item)?;
            }
            let menu_sender = sender.clone();
            let repaint = context.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if let Some((_, action)) =
                    mapping.iter().find(|(identity, _)| *identity == event.id)
                {
                    let _ = menu_sender.send(*action);
                    repaint.request_repaint_of(egui::ViewportId::ROOT);
                }
            }));
            let tray = TrayIconBuilder::new()
                .with_tooltip("麻薯 Mochi · 右键宠物打开面板")
                .with_menu(Box::new(menu))
                .with_icon(Icon::from_rgba(crate::art::tray_rgba(), 32, 32)?)
                .with_menu_on_left_click(true)
                .build()?;
            let hotkeys = GlobalHotKeyManager::new()?;
            let restore_key = HotKey::new(
                Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT),
                Code::KeyR,
            );
            let settings_key = HotKey::new(
                Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT),
                Code::KeyM,
            );
            hotkeys.register(restore_key)?;
            hotkeys.register(settings_key)?;
            GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
                if event.state == HotKeyState::Pressed {
                    let action = if event.id == restore_key.id() {
                        ShellAction::Restore
                    } else {
                        ShellAction::Settings
                    };
                    notify(&sender, &context, action);
                }
            }));
            Ok(Self {
                receiver,
                _tray: tray,
                _hotkeys: hotkeys,
            })
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            let _ = (sender, context);
            Ok(Self { receiver })
        }
    }
}

fn notify(sender: &Sender<ShellAction>, context: &egui::Context, action: ShellAction) {
    let _ = sender.send(action);
    context.request_repaint_of(egui::ViewportId::ROOT);
}
