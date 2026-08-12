# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 椤圭洰姒傝堪

**妗岄潰鍥炬爣鏅鸿兘鍒嗙被绠＄悊宸ュ叿** 鈥?Desktop Organizer

鍩轰簬 Rust 鐨勮法骞冲彴妗岄潰鍥炬爣鏅鸿兘鍒嗙被鍜岀鐞嗗伐鍏枫€俉indows 榛樿鎺у埗闈㈡澘鍜屽垱寤?缂栬緫寮圭獥浣跨敤 **Tauri 2 + Svelte 5**锛汫PUI 涓?`eframe/egui` 浠呬繚鐣欎负鏄惧紡鍥為€€璺緞銆傚垎绫绘鍜屾闈㈣鐩栫獥鍙ｇ户缁娇鐢ㄥ師鐢?Win32 鍒嗗眰绐楀彛 + Direct2D/DirectWrite 娓叉煋锛屼笉杩涘叆 WebView銆?
**鏍稿績鐗规€?*锛氭櫤鑳藉垎绫绘闈㈠浘鏍囥€佸彲瑙嗗寲鍒嗙被妗嗭紙鎷栨嫿/缂╂斁/閿佸畾/鏍峰紡鑷畾涔夛級銆佷竴閿?AI 鍒嗙被銆佸竷灞€璁板繂鎸佷箙鍖栥€佺粺涓€ JSON 閰嶇疆瀵煎叆瀵煎嚭銆?

**骞冲彴鏀寔**锛?
- 鉁?**Windows 10/11**锛氫富瑕佸钩鍙帮紙Shell COM API + Direct2D/DirectWrite 娓叉煋锛?
- 馃搵 **macOS**锛氬緟閲嶆柊瀹炵幇锛堝凡娓呴櫎鏃т唬鐮侊紝shared/ 鍏变韩灞傚凡灏辩华锛屽緟瀹炵幇 DesktopManagerTrait + PlatformService锛?

---

## 鏋勫缓鍜岃繍琛?

```bash
npm --prefix frontend install    # 棣栨瀹夎鍓嶇渚濊禆
npm --prefix frontend run build  # 鏋勫缓宓屽叆寮?Svelte 璧勬簮
cargo run                        # 榛樿 tauri-ui锛岃繍琛屽畬鏁村簲鐢?RUST_LOG=debug cargo run         # 甯﹁皟璇曟棩蹇楄繍琛?cargo run --no-default-features  # legacy GPUI 鍥為€€璺緞锛圵indows锛?cargo build --release            # 鍙戝竷鏋勫缓锛堟棤鎺у埗鍙扮獥鍙ｏ紝windows_subsystem="windows"锛?cargo check                      # 蹇€熺紪璇戞鏌?cargo fmt                        # 鏍煎紡鍖?
cargo clippy                     # Lint 妫€鏌?
cargo test                       # 杩愯娴嬭瘯
cargo watch -x check             # 鑷姩妫€鏌ワ紙闇€ cargo install cargo-watch锛?
```

**鏃ュ織绾у埆**锛氶€氳繃 `RUST_LOG` 鐜鍙橀噺鎺у埗锛坄trace` / `debug` / `info` / `warn` / `error`锛?

**鎺у埗鍙扮獥鍙?*锛氭墍鏈夋瀯寤哄潎閫氳繃 `#![windows_subsystem = "windows"]` 绂佺敤鎺у埗鍙帮紝鏃ュ織鍐欏叆 `~/.ikun_organizer_log/` 鐩綍锛堟寜鏃ユ湡鍜屽簭鍙峰垎鏂囦欢锛?

---

## 鎶€鏈爤

| Crate | 鐗堟湰 | 鐢ㄩ€?|
|-------|------|------|
| tauri / tauri-plugin-opener | 2.x | 榛樿 WebView UI shell銆両PC銆佹寔涔呯獥鍙ｄ笌瀹夊叏澶栭摼 |
| Svelte / Vite / TypeScript | 5.x / 8.x / 6.x | 鎺у埗闈㈡澘銆佸垱寤?缂栬緫绐楀彛鍓嶇 |
| gpui / gpui-component | 0.2.2 / 0.5.1 | legacy Windows UI 鍥為€€璺緞 |
| egui / eframe / epaint | 0.27 | legacy 鎺у埗闈㈡澘鍥為€€璺緞锛坄eframe-ui` feature锛?|
| windows | 0.62 | Windows API锛圫hell COM銆丏irect2D銆丏irectWrite銆丟DI銆佺獥鍙ｇ鐞嗭級 |
| cocoa / objc | 0.25 | macOS API锛堝疄楠屾€э級 |
| serde / serde_json | 1.0 | 缁熶竴 JSON 閰嶇疆搴忓垪鍖?|
| serde_yaml | 0.9 | YAML 鍒嗙被瑙勫垯锛堝吋瀹规棫鏍煎紡锛?|
| anyhow | 1.0 | 閿欒澶勭悊 |
| log / env_logger | 0.4 / 0.11 | 鏃ュ織绯荤粺 |
| uuid | 1.0 | 鍒嗙被妗?/ 鍥炬爣鍞竴 ID |
| chrono | 0.4 | 鏃堕棿澶勭悊 |
| dirs | 5.0 | 璺ㄥ钩鍙伴厤缃洰褰?|
| notify | 6.0 | 鏂囦欢绯荤粺鐩戝惉锛圱ODO锛?|
| lazy_static | 1.0 | 鍏ㄥ眬闈欐€佺姸鎬侊紙澶氶€夈€佹嫋鎷界瓑锛?|
| windows-numerics | 鈥?| D2D 鍚戦噺/鐭╅樀绫诲瀷锛圴ector2 绛夛級 |

---

## 鏋舵瀯姒傝堪锛坴2.0锛?

### 鏍稿績鍘熷垯

- **骞冲彴鎶借薄**锛歚platform/` 閫氳繃 trait 缁熶竴鎺ュ彛锛學indows/macOS 鍚勮嚜瀹炵幇
- **涓氬姟鍒嗙**锛歚box_core/` 鍒嗙被妗嗕笟鍔￠€昏緫涓庡钩鍙版覆鏌撳畬鍏ㄨВ鑰?
- **涓婁笅鏂囨敞鍏?*锛歍auri Commands銆佸悗鍙拌疆璇㈠拰鎻掍欢鍏变韩鍞竴 `SharedTauriRuntime`锛屽叾鍐呴儴鎸佹湁 `AppContext`
- **鎻掍欢鎵╁睍杈圭晫**锛歚PluginRegistry` 宸叉帴鍏?Tauri 鍚姩/閫€鍑猴紝鏀寔杩涚▼鍐?trait 鎻掍欢鐨勮繍琛屾椂瀹夎銆佸嵏杞姐€佸畾鍚戝垎鍙戝拰閫嗗簭娓呯悊锛涗笉鏀寔澶栭儴鍔ㄦ€佸簱 ABI 鐑姞杞?- **鎻掍欢鍔ㄤ綔闂ㄦ帶**锛氭墭鐩樸€佸師鐢熻彍鍗曞拰绗簩瀹炰緥淇″彿瑙﹀彂鐨勬彃浠跺姛鑳藉繀椤诲厛鎴愬姛鍒嗗彂鍛戒护锛涙彃浠跺嵏杞芥垨鍒濆鍖栧け璐ュ悗涓嶅緱缁曡繃娉ㄥ唽琛ㄧ户缁墽琛屽搴斿钩鍙板姩浣?- **缁熶竴閰嶇疆**锛氬崟涓€ `~/.ikun_organizer.json` 绠＄悊鎵€鏈夎缃紝鏀寔瀵煎叆/瀵煎嚭
- **鍏变韩閫昏緫灞?*锛歚shared/` 瀛樻斁浠?Windows 鎻愬彇鐨勫钩鍙版棤鍏抽€昏緫锛宮acOS 閲嶆柊瀹炵幇鏃剁洿鎺ヤ娇鐢?
- **鍗曟枃浠?鈮?1500 琛?*锛氭覆鏌撳紩鎿庣瓑澶嶆潅妯″潡鍙€傚綋鏀惧锛涘彲鐙珛娴嬭瘯鐨勭函閫昏緫搴旀媶鍒板皬妯″潡锛岄伩鍏嶇户缁墿澶ф覆鏌撴枃浠?

### 椤圭洰鐩綍缁撴瀯

```
src/
鈹溾攢鈹€ main.rs                    # 鍏ュ彛锛氶粯璁ゅ惎鍔?Tauri锛屾寜 feature 鍒嗗彂 legacy UI
鈹溾攢鈹€ app.rs                     # legacy eframe::App 鎺у埗闈㈡澘锛堜粎 eframe-ui 鍥為€€璺緞锛?
鈹溾攢鈹€ context.rs                 # AppContext锛堟寔鏈?Box<dyn DesktopManagerTrait> + Box<dyn PlatformService>锛?
鈹?
鈹溾攢鈹€ shared/                    # 馃啎 璺ㄥ钩鍙板叡浜€昏緫灞?
鈹?  鈹溾攢鈹€ mod.rs                 #   妯″潡瀵煎嚭
鈹?  鈹溾攢鈹€ selection.rs           #   鍏ㄥ眬澶氶€夋敞鍐岃〃锛堣法瀹瑰櫒閫夋嫨/閿氱偣/妗嗛€夛級
鈹?  鈹溾攢鈹€ drag_logic.rs          #   鎷栨嫿绾€昏緫锛堟嫋鎷界姸鎬佽绠?shift-insert锛?
鈹?  鈹溾攢鈹€ geometry.rs            #   鍑犱綍璁＄畻锛堝榻愭娴?鍍忕礌杈圭晫/绛夋瘮灞呬腑锛?
鈹?  鈹溾攢鈹€ layout.rs              #   甯冨眬閲嶆帓閫昏緫锛堝垪鏁板彉鍖栨椂鍥炬爣閲嶆槧灏勶級
鈹?  鈹溾攢鈹€ drag_state.rs          #   鎷栨嫿鐢熷懡鍛ㄦ湡鐘舵€佹満
鈹?  鈹斺攢鈹€ types.rs               #   璺ㄥ钩鍙板叡浜被鍨嬪畾涔?
鈹?
鈹溾攢鈹€ tauri_commands/            # Tauri IPC adapter + SharedTauriRuntime
鈹?  鈹溾攢鈹€ config_cmd.rs          #   閫氳繃鍏变韩 AppContext 璇诲啓缁熶竴閰嶇疆
鈹?  鈹溾攢鈹€ box_cmd.rs             #   鍒涘缓/缂栬緫鍘熺敓鍒嗙被妗?鈹?  鈹溾攢鈹€ window_cmd.rs          #   鎸佷箙 WebView 绐楀彛绛栫暐涓庡畨鍏ㄥ閾?鈹?  鈹溾攢鈹€ app_cmd.rs             #   搴旂敤淇℃伅涓庤繘绋嬫帶鍒?鈹?  鈹斺攢鈹€ platform_runtime.rs    #   鍘熺敓鏈嶅姟缂栨帓銆佹彃浠剁敓鍛藉懆鏈熷拰浜嬩欢杞
鈹?鈹溾攢鈹€ gpui_app/                  # legacy Windows GPUI 鍥為€€璺緞
鈹?  鈹溾攢鈹€ mod.rs                 #   妯″潡瀵煎嚭
鈹?  鈹溾攢鈹€ app_runtime.rs         #   UI 妗嗘灦鏃犲叧杩愯鏃?鍛戒护灞傦紙AppContext + 骞冲彴鏈嶅姟锛?
鈹?  鈹溾攢鈹€ control_panel.rs       #   GPUI 鎺у埗闈㈡澘瑙嗗浘
鈹?  鈹溾攢鈹€ create_box_dialog.rs   #   GPUI 鐙珛鈥滃垱寤哄垎绫绘鈥濈獥鍙?
鈹?  鈹溾攢鈹€ windows_main.rs        #   GPUI Application 浜嬩欢寰幆涓庤疆璇㈣皟搴︼紙safe_gpui_update 闃插穿婧冿級
鈹?  鈹斺攢鈹€ settings_view.rs       #   娑叉€佺幓鐠冩晥鏋滆缃潰鏉匡紙GPUI 鐙珛绐楀彛锛?
鈹?
鈹溾攢鈹€ platform/                  # 骞冲彴鎶借薄灞?
鈹?  鈹溾攢鈹€ mod.rs                 #   DesktopManagerTrait(7鏂规硶) + PlatformService(10鏂规硶) + 宸ュ巶鍑芥暟 + re-exports
鈹?  鈹溾攢鈹€ types.rs               #   璺ㄥ钩鍙扮被鍨嬶細DesktopIcon, IconInfo, ScreenInfo
鈹?  鈹溾攢鈹€ windows/
鈹?  鈹?  鈹溾攢鈹€ mod.rs             #   Windows 骞冲彴鍏ュ彛
鈹?  鈹?  鈹溾攢鈹€ ffi_types.rs       #   Win32 FFI 绫诲瀷/甯搁噺
鈹?  鈹?  鈹溾攢鈹€ ffi_imports.rs     #   Win32 extern "system" 澹版槑
鈹?  鈹?  鈹溾攢鈹€ desktop_manager.rs #   妗岄潰鍥炬爣鏋氫妇锛堟枃浠剁郴缁?+ 铏氭嫙 Shell 瀵硅薄 CLSID锛?
鈹?  鈹?  鈹溾攢鈹€ desktop_overlay/   #   妗岄潰瑕嗙洊绐楀彛锛堟覆鏌撴湭绠＄悊鍥炬爣 + Grid 鍚搁檮甯冨眬 + 妗岄潰鎺ョ鐢熷懡鍛ㄦ湡 + 鏂囦欢鐩戝惉鍒锋柊锛?
鈹?  鈹?  鈹?  鈹溾攢鈹€ overlay_mouse_hook.rs # 鍏ㄥ眬榧犳爣閽╁瓙锛圵H_MOUSE_LL + 鐙珛绾跨▼娑堟伅娉碉紝鎷︽埅瑕嗙洊鍖哄煙鍙抽敭锛?
鈹?  鈹?  鈹?  鈹溾攢鈹€ overlay_drop_target.rs # OLE 鎷栨斁鐩爣锛圛DropTarget 瀹炵幇锛?
鈹?  鈹?  鈹溾攢鈹€ alignment_guide/   #   瀵归綈鍙傝€冪嚎锛堝榻愭娴?+ 鍔ㄦ€佸姣旇壊 + overlay 娓叉煋锛?
鈹?  鈹?  鈹溾攢鈹€ d2d_render.rs      #   D2D/DirectWrite 娓叉煋寮曟搸锛坉raw_icon_with_label 缁熶竴鍥炬爣+鏂囨湰娓叉煋锛?
鈹?  鈹?  鈹溾攢鈹€ icon_bitmap.rs     #   鍥炬爣閫忔槑杈圭晫涓庣瓑姣斿眳涓绠楋紙绾€昏緫 + 鍗曞厓娴嬭瘯锛?
鈹?  鈹?  鈹溾攢鈹€ grid_window.rs     #   绐楀彛鍒涘缓/閿€姣?甯冨眬鏀堕泦/Z-order 绠＄悊
鈹?  鈹?  鈹溾攢鈹€ wnd_proc.rs        #   绐楀彛娑堟伅澶勭悊 wnd_proc
鈹?  鈹?  鈹溾攢鈹€ zorder.rs          #   Z-order 灞傜骇绠＄悊锛堝垎绫绘浣庝簬鍏朵粬搴旂敤绐楀彛 + 鍧愭爣鏌ユ壘锛?
鈹?  鈹?  鈹溾攢鈹€ menu_window.rs     #   鑷畾涔夊垎灞傜獥鍙ｆ繁鑹茶彍鍗曪紙D2D/DirectWrite 娓叉煋锛?
鈹?  鈹?  鈹溾攢鈹€ shell_menu.rs      #   Shell 鍙抽敭鑿滃崟锛堝崟閫?澶氶€変笂涓嬫枃鑿滃崟锛? 妗岄潰鑳屾櫙娉ㄥ唽琛ㄨ彍鍗曪紙瑕嗙洊鍖哄煙鍚?Ikun 鍔ㄤ綔锛?
鈹?  鈹?  鈹溾攢鈹€ app_icon.rs        #   宓屽叆 logo.ico 璧勬簮鍔犺浇锛堟墭鐩?鑿滃崟鍥炬爣锛?
鈹?  鈹?  鈹溾攢鈹€ icon_extract.rs    #   鍥炬爣鎻愬彇锛堝唴瀹圭缉鐣ュ浘/搴旂敤 HICON 鍒嗘祦 + .lnk 鐩爣瑙ｆ瀽锛?
鈹?  鈹?  鈹溾攢鈹€ icon_hider.rs      #   鍥炬爣闅愯棌/鎭㈠锛圫ysListView32锛? managed_icons 浠撳簱锛圴ault锛? Staging 鏈哄埗 + 鍒犻櫎鍒嗙被妗嗗悗杩樺師闈炵鐞嗗浘鏍?+ get_vault_display_names
鈹?  鈹?  鈹溾攢鈹€ liquid_glass.rs    #   娑叉€佺幓鐠?GPU 娓叉煋鍣紙DXGI + D2D Effect + HLSL Shader锛?
鈹?  鈹?  鈹溾攢鈹€ multi_select.rs    #   鍏ㄥ眬澶氶€夌姸鎬佺鐞嗭紙璺ㄥ垎绫绘/瑕嗙洊鍖哄煙锛?
鈹?  鈹?  鈹溾攢鈹€ desktop_utils.rs   #   妗岄潰宸ュ叿鍑芥暟锛堣嚜鍔ㄦ帓鍒楁娴?鍥炬爣瀹氫綅锛?
鈹?  鈹?  鈹溾攢鈹€ rename_editor.rs   #   鍐呰仈閲嶅懡鍚嶇紪杈戝櫒锛堢嫭绔嬬嚎绋?EDIT 鎺т欢 + 寮傛娑堟伅閫氫俊 + 鎵╁睍鍚嶄繚鐣欙級
鈹?  鈹?  鈹溾攢鈹€ grid_layout.rs     #   甯冨眬璁＄畻/鎸夐挳鐭╁舰/鍛戒腑娴嬭瘯/婊氬姩閲嶆帓
鈹?  鈹?  鈹溾攢鈹€ monitor_utils.rs   #   鏄剧ず鍣ㄦ煡璇?绐楀彛瀹氫綅宸ュ叿
鈹?  鈹?  鈹溾攢鈹€ drag_image.rs      #   鎷栨嫿娈嬪奖瑕嗙洊绐楀彛锛堝崟閫?澶氶€夊浘鏍囨畫褰憋紝D2D 娓叉煋 + 璺ㄦ樉绀哄櫒 DPI 鑷€傚簲锛?
鈹?  鈹?  鈹溾攢鈹€ ole_dnd.rs         #   OLE 鎷栨嫿婧愶紙IDropSource + IDataObject 瀹炵幇锛?
鈹?  鈹?  鈹溾攢鈹€ drop_target.rs     #   鍒嗙被妗嗘嫋鏀剧洰鏍囷紙IDropTarget 瀹炵幇锛?
鈹?  鈹?  鈹溾攢鈹€ drag_handler.rs    #   鎷栨嫿鍗忚皟鍣紙鎷栨嫿鐢熷懡鍛ㄦ湡锛氶槇鍊兼娴?浣嶇疆鏇存柊/閲婃斁鍒嗗彂锛?
鈹?  鈹?  鈹溾攢鈹€ drag_drop.rs       #   鎷栨嫿鏀剧疆鎿嶄綔锛堝悓妗?璺ㄦ/妗岄潰/OLE drop锛?
鈹?  鈹?  鈹溾攢鈹€ drag_visual.rs     #   鎷栨嫿鍙鍖栧弽棣堬紙鎻掑叆鍒嗛殧绾?鏇挎崲楂樹寒 + 绾€昏緫鍑芥暟锛?
鈹?  鈹?  鈹溾攢鈹€ icon_manager.rs    #   鍥炬爣绠＄悊鍣紙SysListView32 璺ㄨ繘绋嬫搷浣?+ 鍥惧儚鍒楄〃鎻愬彇锛?
鈹?  鈹?  鈹溾攢鈹€ control_panel_window.rs # 鎺у埗闈㈡澘 Win32 鏄剧ず/闅愯棌杈呭姪锛堟墭鐩樹紭鍏堢敓鍛藉懆鏈燂級
鈹?  鈹?  鈹溾攢鈹€ sys_tray.rs        #   绯荤粺鎵樼洏鍏ュ彛锛堥殣钘忔秷鎭獥鍙?+ 鍥哄畾鑿滃崟鍔ㄤ綔闃熷垪锛?
鈹?  鈹?  鈹斺攢鈹€ global_hotkey.rs   #   鍏ㄥ眬鐑敭锛圧egisterHotKey + 娑堟伅绐楀彛 + 蹇嵎閿В鏋愶級
鈹?  鈹斺攢鈹€ macos/
鈹?      鈹溾攢鈹€ mod.rs             #   macOS 骞冲彴鍏ュ彛 + stub 鍑芥暟
鈹?      鈹溾攢鈹€ desktop_manager.rs #   妗岄潰鍥炬爣鏋氫妇锛圢SWorkspace + FileManager锛?
鈹?      鈹溾攢鈹€ ffi_imports.rs     #   macOS FFI 澹版槑锛圕oreGraphics 绛夛級
鈹?      鈹溾攢鈹€ icon_extract.rs    #   鍥炬爣鎻愬彇锛圢SWorkspace iconForFile锛?
鈹?      鈹斺攢鈹€ desktop_overlay/   #   妗岄潰瑕嗙洊绐楀彛
鈹?          鈹溾攢鈹€ mod.rs             #   妯″潡瀵煎嚭
鈹?          鈹溾攢鈹€ overlay_window.rs  #   NSWindow 鍒涘缓/閿€姣?+ 榧犳爣绌块€?
鈹?          鈹溾攢鈹€ overlay_render.rs  #   娓叉煋缂栨帓锛堝垎绫绘 + 鍥炬爣锛?
鈹?          鈹溾攢鈹€ box_render.rs      #   鍒嗙被妗嗘覆鏌擄紙鑳屾櫙/鏍囬/鍥炬爣缃戞牸/鏍囩锛?
鈹?          鈹斺攢鈹€ icon_layout.rs     #   鍥炬爣缃戞牸甯冨眬璁＄畻
鈹?
鈹溾攢鈹€ box_core/                  # 鍒嗙被妗嗘牳蹇冧笟鍔￠€昏緫锛堝钩鍙版棤鍏筹級
鈹?  鈹溾攢鈹€ mod.rs                 #   妯″潡瀵煎嚭
鈹?  鈹溾攢鈹€ box_model.rs           #   ClassificationBox 鏁版嵁妯″瀷 + BoxItem
鈹?  鈹溾攢鈹€ box_manager.rs         #   鍒嗙被妗嗙鐞嗗櫒锛圕RUD + 鎸佷箙鍖栵級
鈹?  鈹溾攢鈹€ sort_engine.rs         #   鎺掑簭寮曟搸锛? 绉嶇淮搴?脳 鍗囬檷搴?
鈹?  鈹溾攢鈹€ view_engine.rs         #   瑙嗗浘寮曟搸锛? 绉嶈鍥炬ā寮忓竷灞€鍙傛暟
鈹?  鈹溾攢鈹€ group_engine.rs        #   鍒嗙粍寮曟搸锛氭寜鍚嶇О/绫诲瀷/鏃堕棿绛?
鈹?  鈹溾攢鈹€ layout_calc.rs         #   缃戞牸甯冨眬璁＄畻 + 鍛戒腑娴嬭瘯
鈹?  鈹斺攢鈹€ interaction.rs         #   浜や簰鐘舵€侊細閫変腑/hover/閿佸畾/鎷栨嫿
鈹?
鈹溾攢鈹€ classification/            # 鍒嗙被绯荤粺
鈹?  鈹溾攢鈹€ mod.rs                 #   AppClassifier锛坒rom_config 浠庨厤缃垱寤猴級
鈹?  鈹溾攢鈹€ database.rs            #   AppCategoryDB锛堜粠閰嶇疆 CategoryRule 鏋勫缓锛?
鈹?  鈹溾攢鈹€ matcher.rs             #   AppMatcher 閫氶厤绗﹀尮閰嶇畻娉?
鈹?  鈹斺攢鈹€ ai_classifier.rs       #   AI 鍒嗙被鍣紙TODO锛歀LM API 璋冪敤锛?
鈹?
鈹溾攢鈹€ config/                    # 缁熶竴閰嶇疆绯荤粺
鈹?  鈹溾攢鈹€ mod.rs                 #   妯″潡瀵煎嚭
鈹?  鈹斺攢鈹€ app_config.rs          #   缁熶竴 JSON 閰嶇疆缁撴瀯锛圓ppConfig锛? 鍗曞厓娴嬭瘯
鈹?
鈹溾攢鈹€ plugin/                    # 鎻掍欢绯荤粺
鈹?  鈹溾攢鈹€ mod.rs                 #   Plugin trait + PluginRegistry
鈹?  鈹斺攢鈹€ builtin/
鈹?      鈹溾攢鈹€ mod.rs             #   register_builtin_plugins()
鈹?      鈹溾攢鈹€ classification_plugin.rs  #   鍒嗙被鍔熻兘鎻掍欢
鈹?      鈹斺攢鈹€ style_plugin.rs    #   鏍峰紡璁剧疆鎻掍欢
鈹?
鈹溾攢鈹€ ui/                        # legacy eframe UI 灞傦紙浠?eframe-ui 鍥為€€璺緞锛?
鈹?  鈹溾攢鈹€ mod.rs                 #   妯″潡瀵煎嚭
鈹?  鈹溾攢鈹€ control_panel.rs       #   鎺у埗闈㈡澘涓荤晫闈?
鈹?  鈹溾攢鈹€ settings_panel.rs      #   璁剧疆闈㈡澘锛堟牱寮?AI/鐑敭/璋冭瘯锛?
鈹?  鈹斺攢鈹€ tray_menu.rs           #   鎵樼洏鑿滃崟閫昏緫
鈹?
鈹溾攢鈹€ utils/                     # 宸ュ叿灞?
鈹?  鈹溾攢鈹€ mod.rs                 #   妯″潡瀵煎嚭
鈹?  鈹溾攢鈹€ logger.rs              #   鏃ュ織绯荤粺锛堟粴鍔ㄦ枃浠惰緭鍑?+ 鑷畾涔夊畯 + 鍙戣鐗堟棩蹇楋級
鈹?  鈹溾攢鈹€ platform.rs            #   璺ㄥ钩鍙板伐鍏凤紙寮€鏈鸿嚜鍚€侀厤缃洰褰曪級
鈹?  鈹溾攢鈹€ dpi.rs                 #   DPI 缂╂斁宸ュ叿
鈹?  鈹斺攢鈹€ string_utils.rs        #   UTF-16 瀛楃涓茶浆鎹?
鈹?
鈹斺攢鈹€ ipc/                       # 杩涚▼闂撮€氫俊锛圱ODO 鍗犱綅锛?    鈹斺攢鈹€ mod.rs                 #   鐑敭鍞ら啋銆佸崟瀹炰緥妫€娴?
```

鍓嶇浣嶄簬 `frontend/src/`锛歚pages/` 璐熻矗涓変釜绐楀彛椤甸潰锛宍components/` 瀛樻斁鏃犱笟鍔＄姸鎬佹帶浠讹紝`lib/api/tauri-api.ts` 鏄敮涓€ IPC adapter銆?
**闈炰唬鐮佺洰褰?*锛?
```
build/
鈹溾攢鈹€ build.rs                   # 宓屽叆 manifest 鍒?Windows 鍙墽琛屾枃浠?
鈹斺攢鈹€ app.manifest               # Per-Monitor DPI V2 鎰熺煡娓呭崟
```


### 渚濊禆瑙勫垯

**鍏佽鐨勪緷璧栨柟鍚?*锛?```
frontend pages 鈫?frontend api adapter 鈫?Tauri Commands
tauri_commands 鈫?AppContext / plugin / platform
legacy UI 鈫?AppContext
context 鈫?config / classification / platform
plugin 鈫?AppContext锛堜粎閫氳繃鐢熷懡鍛ㄦ湡鍙傛暟锛?shared/ 鈫?(鏃犲唴閮ㄤ緷璧栵紝浠?std + serde + uuid + lazy_static)
platform/ 鈫?shared (鍏佽锛氬钩鍙板疄鐜板鐢ㄥ叡浜€昏緫)
```

**绂佹鐨勪緷璧栨柟鍚?*锛?
```
鉂?platform/ 鈫?tauri_commands/ 鎴?frontend/
鉂?config/ 鈫?context/
鉂?box_core/ 鈫?platform/ 鎴?UI shell
鉂?frontend 缁勪欢缁曡繃 tauri-api.ts 鏁ｈ惤 invoke
鉂?shared/ 鈫?platform/    (鍏变韩灞備笉渚濊禆骞冲彴)
鉂?shared/ 鈫?ui/          (鍏变韩灞備笉渚濊禆 UI)
鉂?shared/ 鈫?浠绘剰 UI shell
```

### 鏍稿績璁捐

#### 1. AppContext 鈥?涓績涓婁笅鏂?

```rust
// src/context.rs锛堝疄闄呬唬鐮侊級
pub struct AppContext {
    pub config: AppConfig,
    pub desktop_manager: Box<dyn DesktopManagerTrait>,
    pub platform_service: Box<dyn PlatformService>,
    pub classifier: AppClassifier,
    pub state: AppState,
}
```

榛樿璺緞涓?`TauriRuntime` 鎸佹湁 `AppContext + PluginRegistry`锛屽苟鐢?Tauri `manage()` 娉ㄥ叆涓?`SharedTauriRuntime = Arc<Mutex<TauriRuntime>>`銆傞厤缃?Commands 蹇呴』鏇存柊杩欎竴涓笂涓嬫枃锛岀姝㈡瘡涓懡浠ゅ彟寤哄唴瀛樻ā鍨嬫垨鍙啓纾佺洏銆?
#### 2. 骞冲彴鎶借薄

```rust
// src/platform/mod.rs锛堝疄闄呬唬鐮侊級
pub trait DesktopManagerTrait {
    fn get_desktop_icons(&self) -> Vec<DesktopIcon>;
    fn watch_desktop(&self) -> anyhow::Result<()>;
    fn extract_icon(&self, path: &str) -> anyhow::Result<IconData>;
    fn launch_file(&self, path: &str) -> anyhow::Result<()>;
    // screen / DPI / monitor work-area queries
}
```

`DesktopManagerTrait` 鎶借薄妗岄潰鍥炬爣鑳藉姏锛宍PlatformService` 鎶借薄鎵樼洏銆佸紑鏈哄惎鍔ㄥ拰甯冨眬鏀堕泦銆侱irect2D銆丏XGI銆丼hell/COM 鍜屽師鐢熷垎绫绘浠嶅睘浜庢槑纭殑 Windows adapter锛屼笉鍖呰鎴愪笟鍔℃彃浠躲€?
#### 3. 鎻掍欢绯荤粺

```rust
// src/plugin/mod.rs
pub trait Plugin: Send {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn init(&mut self, ctx: &mut AppContext) -> anyhow::Result<()>;
    fn menu_items(&self) -> Vec<PluginMenuItem>;
    fn handles_command(&self, command: &str) -> bool;
    fn handle_command(&mut self, command: &str, ctx: &mut AppContext) -> anyhow::Result<()>;
    fn cleanup(&mut self, ctx: &mut AppContext);
}
```

`PluginRegistry` 鎷掔粷閲嶅鎻掍欢 ID銆佺┖/鍐茬獊鍛戒护 ID锛沗install()` 鍒濆鍖栨垚鍔熷悗鎵嶅師瀛愬姞鍏ワ紝`uninstall()` 娓呯悊鍚庣Щ闄わ紝`cleanup_all()` 鎸夊畨瑁呴€嗗簭鎵ц銆傚唴缃?`ClassificationPlugin` 涓?`StylePlugin` 鍦?`TauriRuntime::initialize()` 鍒濆鍖栥€傝瑙?`.trellis/spec/backend/plugin-architecture.md`銆?
#### 4. 缁熶竴 JSON 閰嶇疆

`~/.ikun_organizer.json` 绠＄悊鎵€鏈夎缃紝鏀寔瀵煎叆/瀵煎嚭锛?

| 閰嶇疆娈?| 鍐呭 |
|--------|------|
| `general` | 寮€鏈鸿嚜鍚€佽瑷€銆佸崟瀹炰緥銆佸惎鍔ㄦ仮澶嶅竷灞€銆佸惎鍔ㄨ嚜鍔ㄥ垎绫?|
| `hotkeys` | toggle_panel銆乤uto_classify |
| `box_style` | 鑳屾櫙鑹?閫忔槑搴︺€佸渾瑙掋€佸瓧鍙枫€佸浘鏍囧ぇ灏忋€佹爣棰樻爮銆侀攣瀹?鍏抽棴鎸夐挳 |
| `box_defaults` | 鏂板缓鍒嗙被妗嗛粯璁よ鍥?鎺掑簭/鍒嗙粍/灏哄 |
| `ai` | enabled銆乥ase_url銆乤pi_key銆乵odel銆乼emperature銆乼imeout銆乺etries |
| `classification` | 鍒嗙被瑙勫垯鍒楄〃锛圓I 澶辫触鏃剁殑鍏滃簳瑙勫垯锛?|
| `layout` | 鍒嗙被妗嗗垪琛紙浣嶇疆/灏哄/閿佸畾/鎺掑簭/瑙嗗浘/鍒嗙粍/鏂囦欢鍒楄〃/鏍峰紡瑕嗙洊锛?|
| `overlay` | 妗岄潰瑕嗙洊绐楀彛璁剧疆锛堟湭绠＄悊鍥炬爣閫昏緫鍧愭爣銆佸浘鏍囧昂瀵搞€佹帓鍒楁柟鍚戯級 |
| `debug` | log_level銆乴og_to_file銆乻how_fps銆乺ender_wireframe |

**瀛樺偍浣嶇疆**锛歚~/.ikun_organizer.json`锛堢敤鎴蜂富鐩綍锛?

**甯冨眬鎸佷箙鍖栨祦绋?*锛?
```
淇濆瓨: collect_grid_layout() 鈫?config.layout.boxes 鈫?merge_latest_overlay_config() 鈫?save() 鈫?~/.ikun_organizer.json
鎭㈠: load() 鈫?restore_layout() 鈫?hide_native_icons() 鈫?create_desktop_overlay() 鈫?create_grid_window()
鍒犻櫎: 鑿滃崟 CMD_DELETE 鈫?destroy_grid_window(hwnd) 鈫?AppConfig.retain(id != config_id) 鈫?save()
```

**妗岄潰瑕嗙洊绐楀彛鐢熷懡鍛ㄦ湡锛堥樁娈?6/7锛?*锛氬綋 `general.restore_layout_on_start = true` 鏃讹紝搴旂敤鍚姩鍗虫帴绠℃闈㈠浘鏍囩敓鍛藉懆鏈燂細鍏堝垵濮嬪寲鍥炬爣浠撳簱骞堕殣钘忓師鐢?`SysListView32`锛屽啀鍒涘缓 `DesktopOrgOverlay` 娓叉煋鏈繘鍏ュ垎绫绘鐨勫浘鏍囷紝鏈€鍚庢仮澶嶅垎绫绘绐楀彛銆傚嵆浣挎病鏈変繚瀛樼殑鍒嗙被妗嗭紝涔熶細鍒涘缓瑕嗙洊绐楀彛浠ヤ繚鎸佹闈㈡帴绠′竴鑷存€с€傝鐩栫獥鍙ｅ垱寤哄悗浼氬惎鍔ㄦ闈㈡枃浠剁洃鍚紝鐩戝惉鐢ㄦ埛妗岄潰鍜屽叕鍏辨闈㈢洰褰曪紱鏂板銆佸垹闄ゃ€侀噸鍛藉悕鎴栦慨鏀规闈㈡枃浠舵椂缁忚繃绾?500ms debounce 鍚庨噸鏂版灇涓惧苟鍒锋柊鏈鐞嗗浘鏍囥€傞€€鍑烘垨鈥滄竻闄?& 鎭㈠鈥濇椂鎸夐『搴忎繚瀛樺竷灞€銆侀攢姣佸垎绫绘銆佸仠姝㈡闈㈡枃浠剁洃鍚€侀攢姣佽鐩栫獥鍙ｃ€佹仮澶嶅師鐢熷浘鏍囥€佸皢浠撳簱涓殑鍥炬爣杩樺師鍒版闈㈠苟娓呯悊妗岄潰鍙抽敭娉ㄥ唽琛ㄣ€傝繍琛屼腑濡傛灉 Explorer 閲嶅惎瀵艰嚧鍘熺敓妗岄潰鍒楄〃鍙ユ焺澶辨晥鎴栭噸鏂板彲瑙侊紝`native_icons_hidden()` 浼氳Е鍙戦噸鏂伴殣钘忓苟淇濇寔瑕嗙洊绐楀彛杩愯銆?

**瑕嗙洊绐楀彛浣嶇疆鎸佷箙鍖?*锛氳鐩栫獥鍙ｅ浘鏍囦綅缃繚瀛樺湪 `config.overlay.icon_positions`锛岄敭涓烘枃浠惰矾寰勶紝鍊间负 96 DPI 鍩哄噯閫昏緫鍧愭爣銆傝鐩栫獥鍙ｆ嫋鍔ㄥ浘鏍囧悗浠呬繚瀛樼敤鎴锋嫋鎷借瀹氱殑浣嶇疆锛堜笉淇濆瓨鑷姩甯冨眬浣嶇疆锛夛紝閬垮厤鍒锋柊鏃跺浘鏍囬噸鍙狅紱搴旂敤绾т繚瀛樺竷灞€鍓嶅繀椤诲厛鍚堝苟纾佺洏涓婄殑鏈€鏂?`overlay` 閰嶇疆锛岄伩鍏嶇敤鏃х殑鍐呭瓨閰嶇疆瑕嗙洊鏈€鏂颁綅缃€?
**瑕嗙洊绐楀彛鍥炬爣甯冨眬**锛氳鐩栫獥鍙ｄ娇鐢?Grid 缃戞牸甯冨眬锛坥verlay_grid.rs锛夛紝鍥炬爣鎷栨嫿閲婃斁鏃惰嚜鍔ㄥ惛闄勫埌鏈€杩戠殑缃戞牸鏍煎瓙銆傝嫢鐩爣鏍煎瓙宸叉湁鍥炬爣鍒欎氦鎹綅缃紝鏂板浘鏍囧姞鍏ユ椂鑷姩鎵剧涓€涓┖闂叉牸瀛愩€傛墍鏈夌綉鏍艰绠楅€氳繃 overlay_layout_params() 缁熶竴鍙傛暟鍖栵紝鍏煎澶氬睆骞曚笉鍚?DPI/鍒嗚鲸鐜囥€傚垵濮嬪竷灞€浣跨敤绾靛悜浼樺厛锛堝垪浼樺厛锛夛紝鍏堝～婊′竴鍒楀啀鎹笅涓€鍒椼€傛敹闆嗘湭绠＄悊鍥炬爣鏃舵寜 display_name 鍘婚噸锛堝悓涓€鍥炬爣鍙兘鍦ㄧ敤鎴锋闈㈠拰鍏叡妗岄潰鍚勬湁涓€涓?.lnk锛夈€俰s_managed_icon_path 浣跨敤绮剧‘鏂囦欢鍚嶅尮閰嶏紙闈?ends_with锛夛紝閬垮厤鈥滀紒涓氬井淇♀€濆瓙涓茶鍖归厤鈥滃井淇♀€濄€傚垎绫绘鍒犻櫎鍚庨€氳繃 restore_unmanaged_vault_icons() 灏嗕粨搴撲腑涓嶅啀琚鐞嗙殑鍥炬爣杩樺師鍒版闈紝鍐嶉€氳繃 request_desktop_overlay_refresh() 鍒锋柊瑕嗙洊绐楀彛銆?

**UUID 鎸佷箙鍖栨爣璇?*锛氭瘡涓獥鍙ｅ垱寤烘椂鍒嗛厤 `config_id: uuid::Uuid`锛坄GridCreateParams.config_id` 鈫?`GridWindowData.config_id`锛夛紝`collect_grid_layout()` 浣跨敤璇ュ浐瀹?ID锛堣€岄潪姣忔 `Uuid::new_v4()`锛夛紝纭繚鍒犻櫎鎿嶄綔鍙寜 UUID 绮剧‘鍖归厤閰嶇疆鏉＄洰銆?

**DPI 澶勭悊**锛氶厤缃瓨鍌ㄩ€昏緫鍍忕礌锛圖PI 96 鍩哄噯锛夛紝鍒涘缓/鎭㈠鏃?`create_grid_window` 鎸夊疄闄?DPI 缂╂斁锛宍collect_grid_layout` 鍙嶅悜杞崲鍥為€昏緫鍍忕礌銆?

**鍥哄畾琛屼负锛堜唬鐮佺‖缂栫爜锛屼笉鍙厤缃級**锛?
- 濮嬬粓鏄剧ず绯荤粺鎵樼洏锛屼换鍔℃爮涓嶆樉绀?
- 濮嬬粓璁板繂绐楀彛浣嶇疆鍜屽昂瀵?
- 鎵樼洏鍙屽嚮 = 鍛煎嚭鎺у埗闈㈡澘
- 鎵樼洏鍙抽敭 = 鍥哄畾鑿滃崟锛堟墦寮€鎺у埗闈㈡澘 / 鍒涘缓鍒嗙被妗?/ 涓€閿瓵I鍒嗙被 / 閫€鍑猴級

### 娓叉煋鏋舵瀯

- **鍘熺敓妗岄潰娓叉煋**锛氭湭鍒嗙被鍥炬爣鐣欏湪妗岄潰鍘熶綅鐢?Windows 娓叉煋锛屽凡鍒嗙被鍥炬爣绉诲叆浠撳簱鍚庤嚜鐒朵笉鏄剧ず
- **鍥炬爣璧勬簮鍒嗘祦**锛歚platform/windows/icon_extract.rs` 瀵瑰浘鐗囥€佽棰戙€丳DF 绛夊唴瀹规枃浠朵娇鐢?`IShellItemImageFactory` 缂╃暐鍥撅紱瀵?`.lnk`銆乣.exe` 绛夊簲鐢ㄦ枃浠朵娇鐢?`SHDefExtractIconW`/Shell 鍥炬爣锛岄伩鍏?Shell 缂╃暐鍥剧敾甯冧腑鐨勯澶栫暀鐧姐€?
- **鍥炬爣鍐呭閫傞厤**锛歚platform/windows/icon_bitmap.rs` 璐熻矗閫忔槑杈圭晫涓庣瓑姣斿眳涓绠楋紝`d2d_render.rs` 鍦?HICON 杞?D2D 浣嶅浘鏃跺簲鐢ㄨ绛栫暐锛涚綉鏍煎昂瀵镐粛鐢?`box_core::view_engine` 鍜?`grid_layout` 缁熶竴鎺у埗銆?
- **娑叉€佺幓鐠冩晥鏋?*锛氶€氳繃 DXGI Desktop Duplication 鎹曡幏妗岄潰鐢婚潰锛孌2D Effect Graph 澶勭悊绾圭悊锛孒LSL Shader 鐢熸垚棰勪箻 alpha 浣嶅浘
- **瑕嗙洊鍖哄煙浜や簰灞?*锛氶€忔槑鍒嗗眰绐楀彛鎻愪緵澶氶€夈€佸彸閿彍鍗曘€佹嫋鎷界瓑浜や簰鑳藉姏锛堥潪娓叉煋灞傦級

### 渚濊禆鍏崇郴

```
main.rs (Tauri Builder + managed SharedTauriRuntime)
  鈹溾攢鈹€ tauri_commands/ (IPC adapter + 鍘熺敓鏈嶅姟缂栨帓)
  鈹?    鈹溾攢鈹€ platform_runtime.rs (AppContext + PluginRegistry)
  鈹?    鈹溾攢鈹€ config_cmd.rs / box_cmd.rs
  鈹?    鈹斺攢鈹€ window_cmd.rs (鎸佷箙绐楀彛绛栫暐)
  鈹溾攢鈹€ context.rs (AppContext)
  鈹?    鈹溾攢鈹€ config/app_config.rs (AppConfig 缁熶竴閰嶇疆)
  鈹?    鈹溾攢鈹€ platform/ (DesktopManagerTrait + 骞冲彴瀹炵幇)
  鈹?    鈹?    鈹溾攢鈹€ windows/ (D2D娓叉煋 + Win32绐楀彛 + Shell/娉ㄥ唽琛ㄨ彍鍗?+ GPUI 绐楀彛杈呭姪)
  鈹?    鈹?    鈹斺攢鈹€ macos/ (妗岄潰瑕嗙洊绐楀彛 + 鍒嗙被妗嗘覆鏌?+ stub/瀹為獙鎬у姛鑳?
  鈹?    鈹溾攢鈹€ box_core/ (鍒嗙被妗嗕笟鍔￠€昏緫 鈥?骞冲彴鏃犲叧)
  鈹?    鈹溾攢鈹€ classification/ (鍒嗙被瑙勫垯 + AI鍒嗙被)
  鈹溾攢鈹€ plugin/ (杩涚▼鍐呯儹瀹夎/鍗歌浇鐢熷懡鍛ㄦ湡)
  鈹溾攢鈹€ gpui_app/ (legacy Windows UI shell)
  鈹?    鈹溾攢鈹€ app_runtime.rs (UI 妗嗘灦鏃犲叧鍛戒护/杩愯鏃跺眰锛屾寔鏈?AppContext)
  鈹?    鈹溾攢鈹€ control_panel.rs (GPUI 鎺у埗闈㈡澘瑙嗗浘)
  鈹?    鈹溾攢鈹€ create_box_dialog.rs (鐙珛 GPUI 鍒涘缓鍒嗙被妗嗙獥鍙?
  鈹?    鈹溾攢鈹€ edit_box_dialog.rs (鐙珛 GPUI 缂栬緫鍒嗙被妗嗙獥鍙?
  鈹?    鈹斺攢鈹€ windows_main.rs (GPUI 浜嬩欢寰幆)
  鈹溾攢鈹€ app.rs + ui/ (legacy eframe 鍥為€€锛屼粎 eframe-ui feature)
  鈹斺攢鈹€ utils/ (鏃ュ織銆丏PI銆佸瓧绗︿覆)
```

**绾︽潫**锛?
- `box_core/` 涓嶇洿鎺ヨ皟鐢?`platform/` 鈥?閫氳繃 trait 闂存帴璋冪敤
- Tauri Commands 鏄?adapter锛屽彧閫氳繃鍏变韩 `TauriRuntime/AppContext` 鎸佹湁鍙彉閰嶇疆
- Svelte 椤甸潰鍙€氳繃 `frontend/src/lib/api/tauri-api.ts` 璋冪敤 Rust 鍛戒护
- `gpui_app/` 鍜?`ui/` 鏄?legacy 鍥為€€锛屼笉寰楁垚涓洪粯璁よ矾寰勭殑鏂颁緷璧?- `platform/` 鍚勫钩鍙板疄鐜伴€氳繃 `#[cfg]` 闅旂

---

## Windows 骞冲彴鎶€鏈鐐?

**娓叉煋绠＄嚎**锛欴irect2D + DirectWrite 鈫?DIB Section 鈫?`UpdateLayeredWindow()`锛坧er-pixel alpha 鍒嗗眰绐楀彛锛?

**绐楀彛绫诲瀷**锛歚WS_POPUP | WS_EX_LAYERED | WS_EX_TOOLWINDOW`锛屾爣棰樻爮鎷栨嫿閫氳繃 `WM_NCHITTEST 鈫?HTCAPTION`锛岃竟缂樼缉鏀鹃€氳繃 `HTLEFT/HTRIGHT/...`

**鏍囬鏍忓竷灞€**锛氬乏渚х函鏂囨湰鍚嶇О锛堟棤鍥炬爣銆佹棤鏁伴噺锛夛紝鍙充晶 馃敀/馃敁 閿佸畾鎸夐挳 + 鈰?"鏇村"鎸夐挳锛坄Segoe UI Emoji` / `Segoe UI Symbol` 瀛椾綋锛夈€傞攣瀹氭椂鏁翠釜绐楀彛杩斿洖 `HTCLIENT`锛堢姝㈡嫋鎷藉拰缂╂斁锛夛紝鏈攣瀹氭椂姝ｅ父杈圭紭缂╂斁锛坄hover_edge` 椹卞姩钃濊壊鎸囩ず鏉″弽棣堬級銆傛寜閽?hover 鏃舵樉绀烘墜鍨嬪厜鏍?+ 鍗婇€忔槑鑳屾櫙楂樹寒銆?

**鍒嗙被妗嗗悕绉扮紪杈?*锛氬弻鍑绘爣棰樻枃瀛楁墦寮€ GPUI 缂栬緫瀵硅瘽妗嗭紙`edit_box_dialog.rs`锛屾牱寮忎笌"鍒涘缓鍒嗙被妗?瀵硅瘽妗嗗榻愶級銆傞€氳繃 `edit_box_callback` 鍥炶皟浼犻€?`config_id` 鍜屽綋鍓嶅悕绉帮紝鏀寔涓枃 IME 杈撳叆锛堝埄鐢?GPUI 鍐呯疆鐨?`Input` 缁勪欢锛夈€傝緭鍏ラ檺鍒?16 瀛楃銆侲nter 鎴栫偣鍑?纭畾"淇濆瓨锛堣皟鐢?`AppRuntime::rename_box()` 鏇存柊閰嶇疆鍜岀獥鍙ｆ爣棰橈級锛孍scape 鎴栫偣鍑?鍙栨秷"鍏抽棴銆備繚瀛樻椂閫氳繃 `update_grid_window_title()` 瀹炴椂鏇存柊鏍煎瓙绐楀彛鏍囬銆?

**鏇村涓嬫媺鑿滃崟**锛氱偣鍑?鈰?鎸夐挳寮瑰嚭鑷畾涔夊垎灞傜獥鍙ｈ彍鍗曪紙`WS_POPUP | WS_EX_LAYERED` + D2D/DirectWrite 娓叉煋锛夛紝鏀寔 per-pixel alpha 鍗婇€忔槑娣辫壊鑳屾櫙銆佸渾瑙掋€丏irectWrite 娓呮櫚鏂囧瓧锛堜换鎰?DPI 涓嶆ā绯婏級銆佹棤绯荤粺闃村奖銆傚寘鍚垹闄ゃ€佹帓搴忥紙8缁村害+閫掑/閫掑噺锛夈€佸垎缁勶紙8缁村害锛夈€佹煡鐪嬶紙8妯″紡锛岄鐣欏洖璋冩帴鍙ｆ殏鏈疄鐜帮級鍥涗釜鍔熻兘缁勶紝瀛愯彍鍗曡嚜鍔ㄥ畾浣嶏紙灞忓箷杈圭晫缈昏浆锛夈€傚鍒嗙被妗嗚彍鍗曚簰鏂ワ紙鍏ㄥ眬 `OPEN_MENU_HWND` + `PostMessage` 寤惰繜鍒囨崲锛夈€傝彍鍗曟墦寮€鏈熼棿鍒嗙被妗嗚嚜鍔ㄩ攣瀹氥€傞€氳繃 `SetForegroundWindow` + `WM_ACTIVATE(WA_INACTIVE)` 瀹炵幇鐐瑰嚮澶栭儴鍏抽棴锛屽瓙鑿滃崟浣跨敤 `SW_SHOWNOACTIVATE` + `MA_NOACTIVATE` 閬垮厤鎶㈢劍鐐广€?

**閿佸畾鐘舵€佹寔涔呭寲**锛歚GridWindowData.locked` 鈫?`BoxLayoutData.locked` 鈫?`~/.ikun_organizer.json`锛屼繚瀛樻椂 `collect_grid_layout()` 璇诲彇瀹為檯鐘舵€侊紝鎭㈠鏃?`restore_layout()` 浼犲叆 `locked` 鍙傛暟銆?

**绐楀彛灞傜骇**锛?
```
椤跺眰绐楀彛 > 鏅€氬簲鐢?> **鍒嗙被妗嗙獥鍙?* > **妗岄潰瑕嗙洊绐楀彛 DesktopOrgOverlay** > 妗岄潰鍥炬爣 (SysListView32锛岄殣钘? > 澹佺焊 (Progman/WorkerW)
```

**Shell 鍙抽敭鑿滃崟**锛氭枃浠惰彍鍗曚娇鐢?`SHParseDisplayName 鈫?SHBindToParent 鈫?IShellFolder::GetUIObjectOf(IContextMenu)`锛屾闈㈣儗鏅彍鍗曚娇鐢?`IShellFolder 鈫?IShellView::GetItemObject(SVGIO_BACKGROUND)` 鑾峰彇鐪熷疄 Shell 鑿滃崟锛沗InvokeCommand` 浣跨敤 `cmd - FCIDM_SHVIEWFIRST` 浣滀负 verb 鍋忕Щ锛岄伩鍏嶇粷瀵?鐩稿 PIDL 娣风敤銆侰OM 鑿滃崟閾句紭鍏堜娇鐢?`windows` crate 绫诲瀷锛屽け璐ユ椂鍥為€€鍒拌嚜瀹氫箟 `ShellExecuteW` 鑿滃崟銆俙ShellMenuResult` 鍖哄垎 `Cancelled`锛堣彍鍗曟湭鑳芥樉绀猴紝瑙﹀彂鍥為€€锛夊拰 `Dismissed`锛堣彍鍗曞凡鏄剧ず浣嗙敤鎴峰叧闂紝涓嶈Е鍙戝洖閫€锛夛紝閬垮厤鍏抽棴鑿滃崟鍚庤寮瑰洖閫€鑿滃崟銆?

**妗岄潰鍙抽敭鍔ㄤ綔**锛氶€氳繃 Windows 娉ㄥ唽琛?(`HKCU\...\Directory\Background\shell\IkunOpenPanel` / `IkunCreateBox` / `IkunAiClassify` / `IkunQuit`) 娉ㄥ叆鑿滃崟椤癸紝Shell 鑷姩鍚堝苟鍒?`QueryContextMenu` 缁撴灉锛堣鐩栧垵濮嬭彍鍗曞拰鈥滄樉绀烘洿澶氶€夐」鈥漧egacy 鑿滃崟锛夈€傜▼搴忓惎鍔ㄦ椂鍏堟竻鐞嗘棫娈嬬暀鍐嶆敞鍐岋紝閫€鍑烘椂娓呯悊锛岀‘淇濅笉杩愯鏃舵棤娈嬬暀銆傜敤鎴风偣鍑绘椂閫氳繃鍛戒护琛屽弬鏁帮紙濡?`--create-box`锛夊啓鍏ヤ复鏃朵俊鍙锋枃浠讹紱杩愯瀹炰緥鐢?`AppRuntime::tick()` 杞娑堣垂鍚庢墦寮€ GPUI 鎺у埗闈㈡澘銆佺嫭绔嬪垱寤哄垎绫绘绐楀彛銆佹墽琛?AI 鍒嗙被鎴栭€€鍑恒€傚灞忓畾浣嶉€氳繃 Win32 `FindWindowW` + `SetWindowPos` 鐗╃悊鍍忕礌鍧愭爣瀹炵幇銆?

**铏氭嫙 Shell 瀵硅薄锛圕LSID锛?*锛氬洖鏀剁珯銆佹鐢佃剳銆佺綉缁溿€佹帶鍒堕潰鏉跨瓑铏氭嫙瀵硅薄閫氳繃 `::{CLSID}` 璺緞鏍囪瘑銆俙desktop_manager.rs` 缁存姢 `VIRTUAL_SHELL_OBJECTS` 闈欐€佸垪琛紝`clsid_display_name()` 鍑芥暟鎻愪緵澶у皬鍐欎笉鏁忔劅鐨?CLSID 鈫?涓枃鍚嶇О鏄犲皠銆俙BoxLayoutData.display_names` 鎸佷箙鍖栨樉绀哄悕绉帮紝鎭㈠甯冨眬鏃?CLSID 璺緞濮嬬粓閫氳繃 `clsid_display_name()` 瑙ｆ瀽锛堜笉渚濊禆 `display_names` 鏁扮粍锛夛紝鏅€氭枃浠惰矾寰勪紭鍏堜娇鐢?`display_names`銆?

**鍥炬爣鎻愬彇涓庨」鐩浘鏍?*锛歚SHDefExtractIconW` 浼樺厛鎻愬彇 128脳128 楂樻竻鍥炬爣锛宍.lnk` 鍏堥€氳繃 `IShellLink` + `IPersistFile` 瑙ｆ瀽鐩爣鍚庢彁鍙栫洰鏍囧浘鏍囷紝閬垮厤蹇嵎鏂瑰紡绠ご瑕嗗眰姹℃煋鍒嗙被妗嗗浘鏍囥€傚簲鐢ㄩ」鐩浘鏍囦粠 `logo.ico` 缁?`build.rs` 宓屽叆鍙墽琛屾枃浠讹紝渚涚郴缁熸墭鐩樺拰鑿滃崟椤瑰浘鏍囦娇鐢ㄣ€?

**DPI 閫傞厤**锛歮anifest + 杩愯鏃?`SetProcessDpiAwarenessContext(PER_MONITOR_V2)`锛宍GetDpiForWindow` + `WM_DPICHANGED` 鍔ㄦ€佸搷搴?

---

## Legacy GPUI 瀹氭椂鍣ㄩ槻宕╂簝鏈哄埗

璇ユ満鍒朵粎閫傜敤浜?`--no-default-features` 鐨?GPUI 鍥為€€璺緞锛屼笉寰楀鍒跺埌榛樿 Tauri 杩愯鏃躲€侴PUI 鐨?`Entity::update()` / `App::update()` 鍐呴儴浣跨敤鍊熺敤妫€鏌ワ紱寮傛瀹氭椂鍣ㄤ笌娓叉煋寰幆鍐茬獊鏃朵細瑙﹀彂 panic銆?
**瑙ｅ喅鏂规**锛歸indows_main.rs 涓殑 safe_gpui_update() 杈呭姪鍑芥暟閫氳繃 std::panic::catch_unwind 灏?panic 闄嶇骇涓哄彲鎭㈠鐨勮烦杩囨搷浣滐細

- Ok(Ok(T)) 鈥?update 鎴愬姛鎵ц
- Ok(Err(_)) 鈥?GPUI AsyncApp 宸查噴鏀撅紙搴旂敤姝ｅ湪閫€鍑猴級锛岀粓姝㈠畾鏃跺櫒
- Err(RefCellBorrowConflict) 鈥?RefCell 鍊熺敤鍐茬獊锛岃烦杩囨湰娆?tick锛坈ontinue锛?

**娉ㄦ剰**锛氬畾鏃跺櫒闂撮殧宸蹭粠 50ms 璋冩暣涓?100ms锛屾恫鎬佺幓鐠冩覆鏌撳畾鏃跺櫒浠?33ms 璋冩暣涓?100ms锛屽ぇ骞呴檷浣庡啿绐佹鐜囥€?

---

## 寮€鍙戣鑼?

### 閿欒澶勭悊
- 缁熶竴浣跨敤 `anyhow::Result`
- 澶辫触鏃?fallback 鍒伴粯璁ゅ€硷紙涓嶉樆濉炲簲鐢級
- 鎵€鏈夐敊璇繀椤昏褰曟棩蹇?

### 骞冲彴浠ｇ爜
- 浣跨敤 `#[cfg(windows)]` / `#[cfg(target_os = "macos")]` 闅旂
- 閫氳繃 trait 缁熶竴鎺ュ彛

### 浠ｇ爜璐ㄩ噺
- 鎻愪氦鍓?`cargo fmt` + `cargo clippy`
- 鎵€鏈夊叕寮€ API 蹇呴』鏈夋枃妗ｆ敞閲?
- 鍗曟枃浠朵笉瓒呰繃 1500 琛岋紙娓叉煋寮曟搸绛夊鏉傛ā鍧楀彲閫傚綋鏀惧锛?

### 鏃ュ織
- 浣跨敤 `log::{debug, info, warn, error}`
- 鏃ュ織鍐欏叆 `~/.ikun_organizer_log/` 鐩綍锛屾枃浠跺懡鍚嶏細`.ikun_organizer_log_YYYYMMDD_N`锛堝 `.ikun_organizer_log_20260722_1`锛?
- 鍗曟枃浠惰秴杩?2MB 鑷姩鍒囨崲鍒颁笅涓€涓簭鍙锋枃浠?
- 鍚姩鏃惰嚜鍔ㄦ竻鐞嗚秴杩?7 澶╃殑鏃ュ織鏂囦欢
- 鏃ュ織绾у埆閫氳繃 `RUST_LOG` 鐜鍙橀噺鎺у埗
- 寮€鍙戞棩蹇楀畯锛歚log_debug!`銆乣log_info!`銆乣log_warn!`銆乣log_error!`锛堣嚜鍔ㄩ檮鍔犳枃浠?妯″潡淇℃伅锛?
- 鍙戣鐗堟棩蹇楀畯锛歚release_info!`銆乣release_warn!`銆乣release_error!`锛堝甫 `[RELEASE]` 鍓嶇紑锛岀敤浜庣敓浜х幆澧冮棶棰樻帓鏌ワ紝涓嶅簲浣滀负璋冭瘯浠ｇ爜鍒犻櫎锛?
- **窗口上下文标记**：Tauri Command 入口使用 WindowLogGuard::new(window.label()) 自动标记窗口来源（[window:control-panel]、[window:create-box]、[window:edit-box]）
- **前端 IPC 日志**：前端调用 logToRust(level, message) 将日志上报到 Rust 后端，自动标记 [frontend:<window>]
---

## 鍔熻兘闇€姹傦紙寰呭疄鐜帮級

### 鍒嗙被妗嗗姛鑳?
- [x] 鎷栨嫿绉诲姩 + 浣嶇疆璁板繂锛堥€氳繃 collect_grid_layout + restore_layout锛?
- [x] 杈圭紭鎷栨嫿淇敼灏哄 + 灏哄璁板繂锛堝悓涓婏級
- [x] 閿佸畾鎸夐挳 + 閿佸畾璁板繂锛堥攣瀹氬悗涓嶅彲鎷栨嫿/缂╂斁锛宧over 鍙嶉锛岀姸鎬佹寔涔呭寲锛?
- [x] "鏇村"涓嬫媺鑿滃崟锛堟帓搴?鍒嗙粍/鏌ョ湅/鍒犻櫎锛岃嚜瀹氫箟鍒嗗眰绐楀彛 + D2D/DirectWrite 娓叉煋锛屽垹闄ゅ姛鑳藉凡瀹炵幇锛?
- [x] 鍒嗙被妗嗗悕绉颁慨鏀癸紙鍙屽嚮鏍囬鏂囧瓧鎵撳紑 GPUI 缂栬緫瀵硅瘽妗嗭紝杈撳叆妗?+ 纭畾/鍙栨秷鎸夐挳锛?6瀛楃闄愬埗锛孍nter淇濆瓨/Esc鍙栨秷锛?
- [x] 鍥炬爣鍐呰仈閲嶅懡鍚嶏紙Shell 鍙抽敭鑿滃崟 F2 鈫?鐙珛绾跨▼ EDIT 鎺т欢锛岄閫夋枃浠跺悕涓嶅惈鎵╁睍鍚嶏紝寮傛 PostMessage 缁撴灉锛屾墿灞曞悕鑷姩淇濈暀锛?
- [x] 鎺掑簭鏂瑰紡锛氬悕绉般€佺被鍨嬨€佷慨鏀规椂闂淬€佷綔鑰呫€佸垱寤烘椂闂淬€佸ぇ灏忋€佹爣璁般€佹爣棰?脳 閫掑/閫掑噺锛堝紩鎿庡凡瀹炵幇锛岃彍鍗曞彲閫夛級
- [x] 鏌ョ湅鏂瑰紡锛氳秴澶с€佸ぇ銆佷腑銆佸皬銆佸垪琛ㄣ€佽缁嗕俊鎭€佸钩閾恒€佸唴瀹癸紙寮曟搸宸插疄鐜帮紝鑿滃崟鍙€夛級
- [x] 鍒嗙粍灞曠ず锛氭寜鍚嶇О銆佺被鍨嬨€佷慨鏀规椂闂淬€佷綔鑰呫€佸垱寤烘椂闂淬€佸ぇ灏忋€佹爣璁般€佹爣棰橈紙寮曟搸宸插疄鐜帮紝鑿滃崟鍙€夛級
- [x] 鎷栨嫿蹇嵎鏂瑰紡鍒板閮ㄦ闈?/ 澶栭儴妗岄潰蹇嵎鏂瑰紡鎷栧叆鍒嗙被妗嗭紙閫氳繃 DesktopOrgOverlay + Vault 鏃犵鐞嗗憳鍙屽悜绉诲姩锛?
- [x] 鍙抽敭鍔熻兘涓庡師鐢熷揩鎹锋柟寮忎竴鑷达紙Shell 涓婁笅鏂囪彍鍗曞凡瀹炵幇锛?
- [x] 鍒嗙被妗嗗垹闄わ紙鏇村鑿滃崟 鈫?DestroyWindow + UUID 绮剧‘鍒犻櫎閰嶇疆 + rerender_all 淇鍒嗗眰绐楀彛锛?
- [x] 鍥炬爣 Y 杞存粴鍔紙榧犳爣婊氳疆 + 鍗婇€忔槑婊氬姩鏉★紝DPI 鑷€傚簲锛屾粴鍔ㄥ尯鍩熻鍓槻姝㈡孩鍑烘爣棰樻爮锛?
- [x] 绐楀彛灞傜骇绠＄悊锛堝垎绫绘濮嬬粓浣庝簬鍏朵粬搴旂敤绐楀彛锛屽垎绫绘涔嬮棿鐐瑰嚮鐨勫湪鏈€鍓嶏紝WM_WINDOWPOSCHANGING + HWND_BOTTOM + 閫掑綊鎶戝埗锛?
- [x] 鍥炬爣鑷姩閲嶆帓锛堢獥鍙ｅ昂瀵稿彉鍖栨椂鎸夌嚎鎬х储寮曢噸鏂版槧灏勶紝淇濇寔瑙嗚椤哄簭鍜岀┖浣嶏紝鏀寔鎷栨嫿鍚庡浐瀹氫綅缃殑鍔ㄦ€侀噸鎺掞級
- [x] 缁熶竴鍥炬爣鏍峰紡锛堣鐩栧尯鍩熷拰鍒嗙被妗嗕娇鐢ㄧ浉鍚岀殑 hover/閫変腑鑳屾櫙鑹插拰鍦嗚锛屾殫钃濈伆鑹插崐閫忔槑 + 6px 鍦嗚锛?
- [x] 缁熶竴鍥炬爣娓叉煋锛坉raw_icon_with_label 鍏叡鍑芥暟锛屽垎绫绘/瑕嗙洊鍖哄煙/鎷栨嫿娈嬪奖鍏辩敤锛屽瓧浣?鎴柇/闃村奖涓€鑷达級
- [x] 鎷栨嫿鍙鍖栧弽棣堬紙鎻掑叆鍒嗛殧绾?+ 鏇挎崲楂樹寒锛屾敮鎸佸悓妗?璺ㄦ/瑕嗙洊鍖哄煙鎷栨嫿锛宻hift-insert 鏇夸唬 swap锛?
- [x] 鍒嗙被妗嗘嫋鎷芥枃浠跺埌璧勬簮绠＄悊鍣紙OLE CF_HDROP 澶嶅埗鍒?Explorer锛屽浘鏍囦繚鎸佸師浣嶏級
- [x] 娑叉€佺幓鐠冩晥鏋滐紙GPU 娓叉煋鍣?+ 鍙厤缃弬鏁?+ 瀹炴椂璋冭妭鐣岄潰锛?
- [x] 澶氶€夊浘鏍囷紙Ctrl/Shift+鐐瑰嚮 + 妗嗛€?+ 璺ㄥ鍣ㄥ閫?+ 澶氶€夊彸閿彍鍗曪級
- [x] 瑕嗙洊鍖哄煙鎷栨嫿鍒拌祫婧愮鐞嗗櫒锛圤LE CF_HDROP锛?
- [x] 鍒嗙被妗嗗閫夋嫋鎷藉埌璧勬簮绠＄悊鍣?
- [x] 鍥炴敹绔欐枃浠舵嫋鍏ュ垎绫绘锛堣嚜鍔ㄦ仮澶嶅師濮嬫枃浠跺悕锛?
- [x] 鍘熺敓妗岄潰鍥炬爣娓叉煋锛堢Щ闄よ鐩栧尯鍩熺獥鍙ｏ紝鐩存帴浣跨敤鍘熺敓妗岄潰锛?
- [x] 鏂囦欢缂╃暐鍥炬樉绀猴紙IShellItemImageFactory锛?
- [x] 鍏叡妗岄潰鍥炬爣鏀寔锛圥ublic Desktop vault锛?

### 妗岄潰鍙抽敭鑿滃崟
- [x] 鎵撳紑鎺у埗闈㈡澘锛堟敞鍐岃〃娉ㄥ叆 + 淇″彿鏂囦欢 IPC + GPUI 鎺у埗闈㈡澘鏄剧ず锛?
- [x] 鍒涘缓妗岄潰鍒嗙被妗嗭紙娉ㄥ唽琛ㄦ敞鍏?+ 淇″彿鏂囦欢 IPC + 鐙珛 GPUI 鍒涘缓鍒嗙被妗嗙獥鍙ｏ級
- [x] 涓€閿?AI 妗岄潰鍒嗙被锛堟敞鍐岃〃娉ㄥ叆 + 淇″彿鏂囦欢 IPC + 杩愯鏃跺垎绫诲叆鍙ｏ級
- [x] 閫€鍑烘闈㈠垎绫伙紙娉ㄥ唽琛ㄦ敞鍏?+ 淇″彿鏂囦欢 IPC + 娓呯悊閫€鍑猴級

### macOS 骞冲彴锛堝紑鍙戜腑锛?
- [x] 妗岄潰鍥炬爣鏋氫妇锛圢SWorkspace + FileManager API锛?
- [x] 鍥炬爣鎻愬彇锛圢SWorkspace iconForFile + NSImage 鈫?CGImage锛?
- [x] 妗岄潰瑕嗙洊绐楀彛锛圢SWindow + 閫忔槑 + 榧犳爣绌块€忥級
- [x] 鍒嗙被妗嗘覆鏌擄紙鍦嗚鑳屾櫙 + 鏍囬 + 鍥炬爣缃戞牸 + Finder 椋庢牸鏍囩锛?
- [x] 鏍囩鎴柇锛堢涓€琛屾渶澶у墠缂€ + 绗簩琛屼腑闂存埅鏂繚鐣欐墿灞曞悕锛?
- [ ] 鍒嗙被妗嗘嫋鎷?缂╂斁
- [ ] 鍙抽敭鑿滃崟
- [ ] 鍥炬爣鎷栨嫿鍒板垎绫绘
- [ ] 甯冨眬鎸佷箙鍖?

### 鎺у埗鍙帮紙鎺у埗闈㈡澘锛?- [x] 閿洏蹇嵎閿懠鍑?- [x] 涓嶅湪浠诲姟鏍忔樉绀猴紙WS_EX_TOOLWINDOW锛?
- [x] 浠呯郴缁熸墭鐩樺浘鏍囷紙鍙抽敭鑿滃崟锛氭墦寮€闈㈡澘/鍒涘缓鍒嗙被妗?AI鍒嗙被/閫€鍑猴級
- [x] 鍒嗙被妗嗛鏍艰缃紙鑳屾櫙鑹层€佸渾瑙掋€佸浘鏍囧ぇ灏忋€佹枃鏈ぇ灏忥級
- [x] AI BaseUrl / API Key 璁剧疆
- [x] 缁熶竴閰嶇疆鎸佷箙鍖栵紙~/.ikun_organizer.json锛?

---

## 鍙傝€?
- [Tauri 2 鏂囨。](https://v2.tauri.app/)
- [Svelte 5 鏂囨。](https://svelte.dev/docs/svelte/overview)
- [windows-rs](https://github.com/microsoft/windows-rs)
- [Windows Shell API](https://learn.microsoft.com/en-us/windows/win32/shell/)
- 鏋舵瀯閲嶆瀯鏂规锛歚todo/鏋舵瀯閲嶆瀯.md`
- 闇€姹?PRD锛歚todo/desktop-prd.md`



---

## 搴旂敤鍥炬爣瑙勮寖

### logo.ico 瑕佹眰

| 灞炴€?| 瑕佹眰 |
|------|------|
| **浣嶇疆** | 椤圭洰鏍圭洰褰?`logo.ico` |
| **鏍煎紡** | Windows ICO锛堝鍒嗚鲸鐜囧祵鍏ワ級 |
| **蹇呴渶灏哄** | 16脳16, 32脳32, 48脳48, 64脳64, 128脳128, 256脳256 |
| **鑹插僵娣卞害** | 32-bit RGBA |

### 涓轰粈涔堥渶瑕佸鍒嗚鲸鐜?

Windows 鍦ㄤ笉鍚屽満鏅姹備笉鍚屽昂瀵革細绯荤粺鎵樼洏 16脳16銆佹爣棰樻爮 16-32銆佸彸閿彍鍗?16銆佽祫婧愮鐞嗗櫒 32-48銆丄lt+Tab 32-48銆侀珮 DPI 缂╂斁闇€瑕?64/128/256銆傚崟涓€灏哄浼氬鑷?bitmap-stretch 妯＄硦銆?

### 缂栬瘧鏃跺祵鍏ユ満鍒?

- `build.rs` 鐢熸垚 `.rc` 鏂囦欢锛屽紩鐢?`1 ICON "logo.ico"`
- 渚濇灏濊瘯 `windres`锛圡inGW锛夆啋 `rc.exe + cvtres.exe`锛圡SVC锛夆啋 鐩存帴閾炬帴 `.res`
- **璧勬簮缂栬瘧宸ュ叿涓嶅彲鐢ㄦ椂 `panic!` 涓柇缂栬瘧**锛岄槻姝骇鍑烘棤鍥炬爣鐨?.exe
- 缂栬瘧鐜蹇呴』鏈?`windres` 鎴?`rc.exe`锛堝畨瑁?Rust + MinGW 鎴?VS Build Tools锛?

### 杩愯鏃跺姞杞?

- `app_icon.rs` 閫氳繃 `LoadIconW(hinstance, MAKEINTRESOURCE(1))` 浠庡祵鍏ヨ祫婧愬姞杞?
- `get_tray_icon()` 鈫?HICON锛堢郴缁熸墭鐩橈級
- `get_menu_bitmap()` 鈫?HBITMAP锛堝彸閿彍鍗曪級
- `diagnose_icon_availability()` 鈫?鍚姩鏃惰瘖鏂紙鏃ュ織杈撳嚭 `[app_icon]`锛?

### 浣跨敤鍥炬爣鐨勬ā鍧?

| 妯″潡 | 鐢ㄩ€?|
|------|------|
| `sys_tray.rs` | 绯荤粺鎵樼洏鍥炬爣 |
| `shell_menu.rs` | 鍙抽敭鑿滃崟椤?+ 妗岄潰娉ㄥ唽琛ㄨ彍鍗?|
| `grid_window.rs` | 鍒嗙被妗嗙獥鍙ｆ爣棰樻爮 |

### 鏇存崲鍥炬爣

1. 鏇挎崲 `logo.ico`锛堢‘淇濆寘鍚?6 涓繀闇€灏哄锛?
2. `cargo clean` 娓呴櫎缂撳瓨
3. `cargo build` 閲嶆柊缂栬瘧

璇︾粏瑙勮寖瑙?`.trellis/spec/backend/icon-specification.md`銆?

---
