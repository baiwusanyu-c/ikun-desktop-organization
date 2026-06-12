//! 交互状态管理
//!
//! 管理分类框的交互状态：选中、hover、锁定、拖拽、缩放。

/// 分类框交互状态
#[derive(Debug, Clone, Default)]
pub struct InteractionState {
    /// 当前选中项索引（-1 = 无选中）
    pub selected_item: i32,
    /// 当前 hover 项索引（-1 = 无 hover）
    pub hovered_item: i32,
    /// 是否正在拖拽移动
    pub is_dragging: bool,
    /// 是否正在拖拽缩放
    pub is_resizing: bool,
    /// 拖拽起始位置
    pub drag_start: Option<(i32, i32)>,
}

impl InteractionState {
    pub fn new() -> Self {
        Self {
            selected_item: -1,
            hovered_item: -1,
            ..Default::default()
        }
    }

    /// 选中项
    pub fn select(&mut self, index: i32) {
        self.selected_item = index;
    }

    /// 取消选中
    pub fn deselect(&mut self) {
        self.selected_item = -1;
    }

    /// 设置 hover
    pub fn set_hover(&mut self, index: i32) -> bool {
        if self.hovered_item != index {
            self.hovered_item = index;
            true // 需要重绘
        } else {
            false
        }
    }

    /// 是否有选中项
    pub fn has_selection(&self) -> bool {
        self.selected_item >= 0
    }

    /// 开始拖拽
    pub fn start_drag(&mut self, x: i32, y: i32) {
        self.is_dragging = true;
        self.drag_start = Some((x, y));
    }

    /// 结束拖拽
    pub fn end_drag(&mut self) {
        self.is_dragging = false;
        self.drag_start = None;
    }
}
