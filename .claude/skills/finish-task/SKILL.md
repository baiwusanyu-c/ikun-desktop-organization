---
name: finish-task
description: 结束任务，归档，提交代码 - 支持当前任务或所有任务
metadata:
  author: chenzhihui
  version: "1.0.0"
---

# finish-task - 任务完成与归档提交

## 概述

根据用户输入的命令参数，执行任务完成、归档和代码提交流程。

此技能内部调用 `/trellis:finish-work` 进行代码检查。

## 命令触发规则

| 用户输入 | 处理范围 | 执行动作 |
|----------|----------|----------|
| `/finish-task 结束任务，归档，提交` | **仅当前对话上下文的 task** | 1. 调用 `/trellis:finish-work` 检查<br>2. 确定当前任务<br>3. 设置状态为 completed<br>4. 归档任务<br>5. 提交代码 |
| `/finish-task 结束所有任务、归档，提交` | **所有 active tasks** | 1. 调用 `/trellis:finish-work` 检查<br>2. 设置所有 task 状态为 completed<br>3. 归档所有 task<br>4. 提交代码 |

**注意**：此技能是项目级别的，独立于 `/trellis:finish-work` 技能。

---

## 如何识别"当前对话上下文的 task"

**任务存储位置**: `.trellis/tasks/`

通过分析上下文对话内容来确定当前正在处理的任务：

### 识别方法

1. **分析对话历史中的任务提及**
   - 搜索对话中用户明确提及的任务名称或 ID
   - 查找对话开始时的任务上下文（如 Session Start Hook 输出的 `CURRENT TASK` 或 `MY TASKS`）

2. **分析当前 git 分支**
   - 分支名可能包含任务标识（如 `feature/06-04-disabled-btn-tooltip`）
   - 匹配 `.trellis/tasks/` 下的任务目录名

3. **分析最近修改的文件**
   - 查看 `git status` 中的变更文件
   - 根据文件路径推断关联的任务（如修改了 pools 相关文件 → 对应 pools 任务）

4. **检查活跃任务列表**
   - 运行 `python3 ./.trellis/scripts/task.py list` 获取活跃任务
   - 结合对话内容判断哪个任务是当前正在处理的

5. **用户确认**
   - 如果通过以上方法无法唯一确定，询问用户确认要结束哪个任务

### 任务信息来源

- **Session Start Hook 输出**: 包含 `CURRENT TASK`、`MY TASKS` 等信息
- **对话中的用户指令**: 用户可能在对话中明确指定了任务
- **任务目录**: `.trellis/tasks/{MM-DD}-{name}/` 包含 `task.json` 和 `prd.md`

---

## 执行流程

### 步骤 1: 调用 trellis:finish-work 进行检查

```typescript
// 调用 trellis:finish-work 技能执行代码检查
Skill({ skill: "trellis:finish-work", args: "结束任务并归档提交" })
```

等待检查完成后，继续后续步骤。

### 步骤 2: 确定当前任务

**仅当前任务时**：

从对话上下文中分析并确定当前任务：
- 检查对话开始时的 Session Start Hook 输出中的 `CURRENT TASK` 或 `MY TASKS`
- 分析对话历史中用户提及的任务相关信息
- 查看 `.trellis/tasks/` 下活跃任务的状态

**所有任务时**：

获取所有活跃任务列表：
```bash
python3 ./.trellis/scripts/task.py list
```

### 步骤 3: 更新任务状态

**仅当前任务时**：
```bash
# 更新指定任务的 task.json
python3 ./.trellis/scripts/task.py update <task-dir> --status completed
```

**所有任务时**：
```bash
# 遍历 .trellis/tasks/ 下所有活跃任务，逐个更新状态
for task in active_tasks:
    python3 ./.trellis/scripts/task.py update <task-dir> --status completed
```

### 步骤 4: 归档任务

**仅当前任务时**：
```bash
# 将任务目录移动到 .trellis/tasks/archive/{year-month}/
python3 ./.trellis/scripts/task.py archive <task-name>
```

**所有任务时**：
```bash
# 遍历所有活跃任务并归档
for task in active_tasks:
    python3 ./.trellis/scripts/task.py archive <task-name>
```

### 步骤 5: 提交代码

```bash
# 1. 查看变更
git status
git diff --name-only

# 2. 添加变更文件
git add <changed-files>

# 3. 提交代码
git commit -m "type(scope): description"
```

**提交信息格式**: `feat|fix|docs|refactor|test|chore(scope): description`

示例：
- `feat(pools): 商品清单表格支持跨页保持勾选状态`
- `fix(button): 修复禁用按钮 tooltip 提示`

---

## 注意事项

1. **区分处理范围**：必须严格按照用户输入判断处理范围
   - `结束任务，归档，提交` → 仅当前任务
   - `结束所有任务、归档，提交` → 所有任务

2. **从对话上下文分析任务**：通过 Session Start Hook 输出、对话历史、git 状态等多维度分析确定当前任务

3. **确认当前任务**：无法唯一确定当前任务时，必须询问用户

4. **归档顺序**：归档任务后再提交代码，避免遗漏

5. **错误处理**：如果任何步骤失败，停止流程并报告错误

---

## 与 trellis:finish-work 的关系

| 技能 | 用途 | 关系 |
|------|------|------|
| `/trellis:finish-work` | 代码完成度检查清单 | 被 `/finish-task` 调用 |
| `/finish-task` | 任务完成 + 归档 + 提交 | 内部调用 `/trellis:finish-work` |

**执行顺序**：
```
/finish-task
    │
    ▼
调用 /trellis:finish-work (代码检查)
    │
    ▼
确定当前任务 (从对话上下文分析)
    │
    ▼
更新任务状态 + 归档
    │
    ▼
提交代码
```