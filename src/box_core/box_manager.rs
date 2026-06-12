//! 分类框管理器
//!
//! 管理所有分类框的生命周期：创建、删除、查找、持久化。

use super::box_model::ClassificationBox;
use uuid::Uuid;

/// 分类框管理器
pub struct BoxManager {
    /// 所有分类框
    boxes: Vec<ClassificationBox>,
}

impl Default for BoxManager {
    fn default() -> Self {
        Self { boxes: Vec::new() }
    }
}

impl BoxManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// 创建分类框
    pub fn create_box(&mut self, name: impl Into<String>) -> &ClassificationBox {
        let b = ClassificationBox::new(name);
        self.boxes.push(b);
        self.boxes.last().unwrap()
    }

    /// 删除分类框
    pub fn delete_box(&mut self, id: Uuid) -> bool {
        let len_before = self.boxes.len();
        self.boxes.retain(|b| b.id != id);
        self.boxes.len() < len_before
    }

    /// 按 ID 查找分类框
    pub fn find_box(&self, id: Uuid) -> Option<&ClassificationBox> {
        self.boxes.iter().find(|b| b.id == id)
    }

    /// 按 ID 查找分类框（可变）
    pub fn find_box_mut(&mut self, id: Uuid) -> Option<&mut ClassificationBox> {
        self.boxes.iter_mut().find(|b| b.id == id)
    }

    /// 获取所有分类框
    pub fn all_boxes(&self) -> &[ClassificationBox] {
        &self.boxes
    }

    /// 获取分类框数量
    pub fn count(&self) -> usize {
        self.boxes.len()
    }

    // TODO: 从配置加载布局
    // pub fn load_from_config(&mut self, layout: &LayoutConfig) { ... }

    // TODO: 保存布局到配置
    // pub fn save_to_config(&self, layout: &mut LayoutConfig) { ... }
}
