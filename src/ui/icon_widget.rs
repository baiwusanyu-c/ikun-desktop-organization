//! 图标组件
//!
//! 单个桌面图标的显示组件

use eframe::egui;
use crate::desktop::DesktopIcon;

/// 图标组件
pub struct IconWidget {
    /// 图标数据
    pub icon: DesktopIcon,
    /// 是否选中
    pub selected: bool,
}

impl IconWidget {
    pub fn new(icon: DesktopIcon) -> Self {
        Self {
            icon,
            selected: false,
        }
    }

    /// 显示图标
    pub fn show(&self, ui: &mut egui::Ui) -> egui::Response {
        // 图标按钮
        let response = ui.selectable_label(
            self.selected,
            &self.icon.display_name,
        );

        // TODO: 添加实际图标图片显示
        // TODO: 添加双击打开功能
        // TODO: 添加拖拽移动功能

        response
    }

    /// 设置选中状态
    pub fn set_selected(&mut self, selected: bool) {
        self.selected = selected;
    }
}