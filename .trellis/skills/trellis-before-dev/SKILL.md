---
name: trellis-before-dev
description: 开发前规范读取流程，发现并阅读相关代码规范文档，确保编码前了解项目约定
metadata:
  author: trellis
  version: "1.0.0"
  compatibility:
    - codex
    - hermes
    - claude-code
---

# trellis-before-dev - 开发前规范读取

## 概述

在开始编码任务前，发现并阅读相关的开发规范文档，确保遵循项目约定。

当用户说"开始开发"、"准备编码"、"阅读规范"、或准备实现某个功能时触发。

**此步骤是编码前的强制要求。**

---

## 执行步骤

### 步骤 1：发现包和规范层级

```bash
python3 ./.trellis/scripts/get_context.py --mode packages
```

这会显示项目中的包及其规范层级。

### 步骤 2：识别适用的规范

根据任务判断哪些规范适用：
- 你正在修改哪个包（如 `cli/`、`docs-site/`）
- 工作类型（backend、frontend、unit-test、docs 等）

### 步骤 3：阅读规范索引

读取每个相关模块的规范索引：

```bash
cat .trellis/spec/<package>/index.md
```

按照索引中的「Pre-Development Checklist」部分操作。

### 步骤 4：阅读具体规范文件

索引文件指向具体的规范文档（如 `error-handling.md`、`conventions.md`、`mock-strategies.md`）。

**读取这些文件**以了解编码标准和模式：

```bash
cat .trellis/spec/<package>/error-handling.md
cat .trellis/spec/<package>/conventions.md
# 等等 — 根据 Pre-Development Checklist 列出的内容
```

### 步骤 5：始终阅读共享指南

```bash
cat .trellis/spec/guides/index.md
```

---

## 任务类型与必读文档

| 任务类型 | 必读文档 |
|----------|----------|
| 前端工作 | `frontend/index.md` → 相关文档 |
| 后端工作 | `backend/index.md` → 相关文档 |
| 跨层功能 | `guides/cross-layer-thinking-guide.md` |

---

## 理解编码标准

阅读规范后，确保理解：
- 该任务需要遵循的编码标准和模式
- 文件组织约定
- 错误处理方式
- 测试要求

然后继续制定开发计划。

---

## 注意

**编码前必须执行此步骤。** 跳过规范阅读会导致代码不符合项目约定。
