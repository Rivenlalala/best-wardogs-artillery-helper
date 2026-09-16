# best-wardogs-artillery-helper

Wardogs 炮兵解算助手：从游戏聊天框复制坐标 → 算出距离/方位角/密位 → 在屏幕上画出
「幽灵刻度」，玩家把游戏自己的密位带和罗盘条对上去就完事。

**不读屏幕、不注入按键、不 hook 游戏。** 只有剪贴板读取 + 一层点击穿透的
`WS_EX_LAYERED | TRANSPARENT | NOACTIVATE | TOPMOST` 覆盖窗口。

术语见 [CONTEXT.md](CONTEXT.md)。

## 结构

| 路径 | 内容 |
|------|------|
| `core/` | 纯解算：坐标解析、火表插值、方位角。跨平台，有测试。 |
| `app/`  | Windows 控制面板 + 覆盖层（eframe + Win32）。独立 workspace。 |
| `core/data/weapons.json` | 火表：L81 迫击炮、SPH-2 SPG（低角/高角）。 |

## 用法

1. 运行 `autoartillery-app.exe`（角落小面板，不抢焦点）。
2. 游戏里复制自己炮位的坐标 —— 第一次复制的是 **原点**，一直沿用，换炮位按 `RESET`。
3. 复制目标坐标，面板出读数，覆盖层出幽灵刻度。
4. 选炮种：`MORTAR` / `SPG · LOW` / `SPG · HIGH`。

覆盖层的几何常数是按 **3840x2160** 反推的，分辨率不符会显式告警。

## 构建

测试解算核心（任意平台）：

```bash
cargo test
```

从 Linux/WSL 交叉编译 Windows 可执行文件（需 `cargo-xwin` + `clang lld llvm`）：

```bash
cd app && cargo xwin build --release --target x86_64-pc-windows-msvc
# -> app/target/x86_64-pc-windows-msvc/release/autoartillery-app.exe
```

在 Windows 上直接构建：`cd app && cargo build --release`。

## 发布

```bash
cd app && cargo xwin build --release --target x86_64-pc-windows-msvc
gh release create v0.1.0 app/target/x86_64-pc-windows-msvc/release/autoartillery-app.exe
```

## 问题追踪

用 [beads](https://github.com/gastownhall/beads)：`bd ready` / `bd show <id>`。
