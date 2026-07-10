---
name: trellis-start
description: 初始化 Trellis 开发会话，获取上下文，分类任务并启动工作流
metadata:
  author: trellis
  version: "1.0.0"
  compatibility:
    - codex
    - hermes
    - claude-code
---

# trellis-start - 会话初始化

## 概述

初始化 AI 开发会话，获取项目上下文，分类用户任务并启动相应工作流。

当用户描述要开始一个开发任务、开始新会话、或说"开始工作"时触发。

---

## 执行步骤

### 步骤 1：理解开发工作流

读取工作流指南：

```bash
cat .trellis/workflow.md
```

### 步骤 2：获取当前上下文

```bash
python3 ./.trellis/scripts/get_context.py
```

这会显示：开发者身份、git 状态、当前任务（如有）、活跃任务列表。

### 步骤 3：读取规范索引

```bash
python3 ./.trellis/scripts/get_context.py --mode packages
```

查看可用的包和规范层级，读取相关规范索引：

```bash
cat .trellis/spec/<package>/index.md        # 包级别规范
cat .trellis/spec/guides/index.md           # 通用指南（必读）
```

> **重要**：索引文件是导航页。开始实际开发前，**必须**读取索引中「Pre-Development Checklist」列出的具体规范文件。

### 步骤 4：报告状态并询问

向用户报告已获取的上下文，然后询问："你想要做什么？"

---

## 任务分类

根据用户描述分类：

| 类型 | 判断标准 | 处理方式 |
|------|----------|----------|
| **问题** | 询问代码、架构 | 直接回答 |
| **简单修复** | 拼写修复、单行变更 | 直接编辑 |
| **简单任务** | 目标明确、1-2 文件 | 快速确认 → 实现 |
| **复杂任务** | 目标模糊、多文件 | **需求发现 → 任务工作流** |

### 简单任务处理

快速确认后，**不中断地执行所有步骤**：
1. 创建任务目录
2. 编写 PRD
3. 研究代码库
4. 配置上下文
5. 激活任务
6. 实现代码
7. 质量检查
8. 完成

### 复杂任务处理

自动启动需求发现流程（参见 `trellis-brainstorm` Skill）：
1. 确认理解
2. 创建任务目录
3. 逐个提问
4. 提出方案
5. 确认最终需求
6. 进入任务工作流

### 继续已有任务

如果 `get_context.py` 显示有当前任务：
1. 读取任务的 `prd.md` 了解目标
2. 检查 `task.json` 的状态和阶段
3. 询问用户："继续处理 <task-name>？"

---

## 任务工作流（开发任务）

### Phase 1：创建任务

```bash
TASK_DIR=$(python3 ./.trellis/scripts/task.py create "<标题>" --slug <slug>)
```

编写 `prd.md`：

```markdown
# <任务标题>

## 目标
<要实现什么>

## 需求
- <需求 1>
- <需求 2>

## 验收标准
- [ ] <标准 1>

## 技术笔记
<技术决策或约束>
```

### Phase 2：准备实现

**研究代码库**（调用 research 子 Agent，如有 subagent/task 工具）：

```
Task(
  subagent_type: "research",
  prompt: "分析代码库：任务: <PRD 目标>, 类型: <frontend/backend/fullstack>
  查找：1. 相关 .trellis/spec/ 文件 2. 现有代码模式 3. 可能需修改的文件"
)
```

**配置上下文**：

```bash
python3 ./.trellis/scripts/task.py init-context "$TASK_DIR" <type>
python3 ./.trellis/scripts/task.py add-context "$TASK_DIR" implement "<path>" "<reason>"
python3 ./.trellis/scripts/task.py add-context "$TASK_DIR" check "<path>" "<reason>"
```

**激活任务**：

```bash
python3 ./.trellis/scripts/task.py start "$TASK_DIR"
```

### Phase 3：执行

**实现**（调用 implement 子 Agent）：

```
Task(
  subagent_type: "implement",
  prompt: "实现 prd.md 中描述的任务。遵循注入的代码规范。完成前运行 lint 和 typecheck。"
)
```

**检查**（调用 check 子 Agent）：

```
Task(
  subagent_type: "check",
  prompt: "根据代码规范审查所有变更。修复发现的问题。确保 lint 和 typecheck 通过。"
)
```

### Phase 4：完成

1. 确认 lint 和 typecheck 通过
2. 报告实现内容
3. 提醒用户测试并提交

---

## 脚本命令参考

| 命令 | 用途 |
|------|------|
| `python3 ./.trellis/scripts/get_context.py` | 获取完整会话上下文 |
| `python3 ./.trellis/scripts/task.py create "<title>"` | 创建新任务 |
| `python3 ./.trellis/scripts/task.py init-context <dir> <type>` | 初始化 jsonl 上下文 |
| `python3 ./.trellis/scripts/task.py add-context <dir> <agent> <path> <reason>` | 添加代码规范到上下文 |
| `python3 ./.trellis/scripts/task.py start <name>` | 设置为当前任务 |
| `python3 ./.trellis/scripts/task.py finish` | 清除当前任务 |
| `python3 ./.trellis/scripts/task.py archive <name>` | 归档已完成的任务 |
| `python3 ./.trellis/scripts/add_session.py` | 记录会话进度 |
