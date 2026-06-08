# RustRover 配置修复说明

## 问题

遇到错误：
```
Unknown run configuration type CargoCommandRunConfiguration
```

## 原因

运行配置文件放在了错误的位置：
- ❌ `.run/` 目录（错误）
- ✅ `.idea/runConfigurations/` 目录（正确）

## 已修复

### 1. 删除错误配置

```bash
rm -rf .run/
```

### 2. 创建正确配置

在 `.idea/runConfigurations/` 目录创建 4 个配置文件：

| 文件 | 用途 | 环境变量 |
|------|------|----------|
| `Debug.xml` | 调试运行 | `RUST_LOG=debug` |
| `Run.xml` | 正常运行 | `RUST_LOG=info` |
| `Check.xml` | 快速检查 | - |
| `Test.xml` | 运行测试 | `RUST_LOG=debug` |

### 3. 配置文件格式

正确的 RustRover 配置格式：

```xml
<component name="ProjectRunConfigurationManager">
  <configuration default="false" name="Debug" type="CargoCommandRunConfiguration" factoryName="Cargo Command">
    <option name="command" value="run" />
    <option name="workingDirectory" value="$PROJECT_DIR$" />
    <envs>
      <env name="RUST_LOG" value="debug" />
    </envs>
    <method v="2">
      <option name="CargoCommandRunConfiguration" enabled="true" />
    </method>
  </configuration>
</component>
```

### 4. 使用方法

1. 在 RustRover 中重新加载项目：
   - `File > Reload from Disk`
   - 或关闭并重新打开项目

2. 查看运行配置：
   - 右上角下拉框应显示：Run, Debug, Check, Test

3. 使用配置：
   - 选择 **Debug** 配置
   - 按 `Cmd+D`（macOS）或 `Ctrl+D`（Windows）
   - 应用启动，日志显示在 Run 面板

## 验证

```bash
ls .idea/runConfigurations/
# 应显示：Check.xml Debug.xml Run.xml Test.xml
```

## 注意事项

如果 RustRover 仍然无法识别配置：

1. **手动创建配置**（推荐）：
   - 点击右上角 "Edit Configurations..."
   - 点击 "+" 添加新配置
   - 选择 "Cargo Command"
   - 设置：
     - Name: `Debug`
     - Command: `run`
     - Environment variables: `RUST_LOG=debug`
   - 点击 OK

2. **重新加载项目**：
   - `File > Invalidate Caches...`
   - 选择 "Invalidate and Restart"

## 手动创建步骤（最可靠）

如果配置文件仍然有问题，建议手动创建：

### Debug 配置

1. RustRover → 右上角 "Edit Configurations..."
2. 点击 "+" → 选择 "Cargo Command"
3. 设置：
   - **Name**: `Debug`
   - **Command**: `run`
   - **Working directory**: `$PROJECT_DIR$`
   - **Environment variables**: 点击右侧图标 → 添加 `RUST_LOG=debug`
4. 点击 "Apply" 和 "OK"

### Run 配置

1. 同上步骤
2. 设置：
   - **Name**: `Run`
   - **Command**: `run`
   - **Environment variables**: `RUST_LOG=info`

### Check 配置

1. 同上步骤
2. 设置：
   - **Name**: `Check`
   - **Command**: `check`

### Test 配置

1. 同上步骤
2. 设置：
   - **Name**: `Test`
   - **Command**: `test`
   - **Environment variables**: `RUST_LOG=debug`

---

**修复完成时间**：2026-06-08