# 进程内插件架构

## 边界

`src/plugin/mod.rs` 提供编译进应用的 trait 插件机制。它支持进程运行期间安装、卸载、初始化、命令分发和清理，因此属于“进程内热拔插”；它不扫描插件目录、不加载 DLL，也不承诺 Rust trait object 的跨版本 ABI。

平台渲染、Win32 窗口、Shell/COM、托盘和 DXGI 必须保留在 `src/platform/`。插件可以通过生命周期参数访问 `AppContext`，但 `platform/` 不得反向依赖 `plugin/`。

## 运行时所有权

```text
main.rs
  -> SharedTauriRuntime = Arc<Mutex<TauriRuntime>>
       -> AppContext
       -> PluginRegistry
            -> PluginEntry { plugin: Box<dyn Plugin>, initialized: bool }
```

`src/tauri_commands/platform_runtime.rs` 的 `TauriRuntime::initialize` 调用 `PluginRegistry::init_all`；退出线程在销毁平台资源前调用 `cleanup_all`。

由插件拥有的运行时动作必须先成功完成 `dispatch_command`。托盘、原生菜单和第二实例信号统一进入 `request_create_box` / `request_auto_classify`；插件未初始化、已卸载或命令处理失败时，不得继续执行对应平台动作。这样 `uninstall` 才会立即撤销插件能力，而不是只移除菜单展示。

## API 契约

| API | 输入 | 成功状态 | 失败/基础状态 |
| --- | --- | --- | --- |
| `register(plugin)` | `Box<dyn Plugin>` | 加入未初始化条目 | 空/重复插件 ID、空/重复/冲突命令 ID 返回 `Err` |
| `init_all(ctx)` | `&mut AppContext` | 成功条目标为 initialized | 单插件失败记录到 `PluginInitReport.failed`，继续初始化其它插件 |
| `install(plugin, ctx)` | plugin + context | 初始化成功后原子加入 | init 失败不加入注册表 |
| `uninstall(id, ctx)` | 插件 ID | 已初始化插件先 cleanup 再移除 | 未找到返回 `false` |
| `dispatch_command(command, ctx)` | 命令 ID | 只调用第一个声明处理该命令的已初始化插件 | 未处理返回 `Ok(false)`，插件错误增加上下文后返回 |
| `cleanup_all(ctx)` | context | 按安装逆序 cleanup，并重置 initialized | 未初始化条目跳过 |

`Plugin::handles_command` 默认根据 `menu_items().id` 判断；插件若处理不暴露在菜单中的内部命令，可以显式覆盖，但必须保证命令命名空间唯一。

## 生命周期状态

```text
register -> registered/uninitialized -> init_all -> initialized
install  -------------------------------> initialized
initialized -> uninstall -> cleanup -> removed
initialized -> process shutdown -> cleanup_all -> registered/uninitialized
```

同一插件的 `init` 在未 cleanup 前最多成功一次。批量初始化失败的插件保留为 uninitialized，允许后续重试或卸载。

## Good / Base / Bad

- Good：内置插件在 `AppContext` 创建后初始化，退出时逆序清理。
- Good：运行时 `install` 成功后菜单和命令立即可用，`uninstall` 后立即不可分发。
- Good：同一功能的托盘、原生菜单和第二实例信号入口都经过同一插件命令门控。
- Base：一个插件 init 失败，其他插件仍启动，失败插件不暴露菜单项。
- Base：未知命令返回 `Ok(false)`，不广播给全部插件。
- Bad：重复插件 ID 或命令 ID 必须在注册前拒绝，不能覆盖既有插件。
- Bad：init 失败的运行时插件不得残留半初始化条目。
- Bad：插件分发返回 `false` 或 `Err` 后不得绕过注册表直接执行其平台动作。
- Bad：不得把任意 DLL 路径或前端输入转换成 `Box<dyn Plugin>`。

## 必需测试

`src/plugin/mod.rs` 的单元测试至少覆盖：

- 重复插件 ID 与命令冲突拒绝；
- `install -> dispatch -> uninstall` 顺序及 cleanup；
- init 失败的原子性；
- 多插件逆序清理；
- 未知命令不被广播。

提交前运行 `cargo test plugin::tests` 和完整 `cargo test`。
