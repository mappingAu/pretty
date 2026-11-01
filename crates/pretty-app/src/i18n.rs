//! Small, dependency-free UI localisation layer.
//!
//! English strings are used as stable keys so the translation table is easy to
//! audit. The terminal protocol and log messages stay in English; this covers
//! the human-facing window, settings, context menu and drop hint.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    English,
    Chinese,
}

impl Lang {
    pub fn from_setting(value: &str) -> Self {
        match value {
            "en" | "en-US" | "en_US" => Self::English,
            "zh" | "zh-CN" | "zh_CN" | "zh-Hans" => Self::Chinese,
            _ => Self::detect(),
        }
    }

    pub fn detect() -> Self {
        let locale = std::env::var("LC_ALL")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var("LC_MESSAGES").ok().filter(|s| !s.is_empty()))
            .or_else(|| std::env::var("LANG").ok().filter(|s| !s.is_empty()))
            .unwrap_or_default();
        if locale.to_ascii_lowercase().contains("zh") {
            Self::Chinese
        } else {
            Self::English
        }
    }


    pub fn native_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Chinese => "简体中文",
        }
    }
}

/// Translate an English UI key. Unknown keys fall back to English.
pub fn tr(lang: Lang, key: &'static str) -> &'static str {
    if lang == Lang::English {
        return key;
    }
    match key {
        "Settings" => "设置",
        "Open Pretty settings (Ctrl+,)" => "打开 Pretty 设置（Ctrl+,）",
        "Pretty Settings" => "Pretty 设置",
        "General" => "通用",
        "Appearance" => "外观",
        "Rendering" => "渲染",
        "Keyboard" => "键盘",
        "About" => "关于",
        "Save now" => "立即保存",
        "Changes are applied live and saved automatically." => "更改实时生效并自动保存。",
        "Language" => "语言",
        "Automatic (system)" => "自动（系统）",
        "Font family" => "字体",
        "Font size" => "字号",
        "Line height" => "行高",
        "Ligatures" => "连字",
        "Enable programming ligatures (liga/calt)" => "启用编程连字（liga/calt）",
        "OpenType features" => "OpenType 特性",
        "Scrollback" => "回滚缓冲",
        "Shell" => "Shell",
        "Cursor" => "光标",
        "Blinking cursor" => "光标闪烁",
        "Cell grid" => "单元格网格",
        "Opacity" => "不透明度",
        "Compositor blur" => "合成器模糊",
        "Blur desktop behind the window (KWin/X11)" => "模糊窗口后的桌面（KWin/X11）",
        "Padding" => "内边距",
        "Tiling gap" => "平铺间距",
        "Background" => "背景",
        "Foreground" => "前景",
        "Accent" => "强调色",
        "Selection background" => "选区背景",
        "Selection foreground" => "选区前景",
        "Configuration UI" => "配置界面",
        "The settings overlay is themed separately and can be customised:" => "设置浮层有独立主题，可单独定制：",
        "UI accent" => "界面强调色",
        "UI panel opacity" => "界面面板不透明度",
        "Prefer X11/XWayland for compositor blur (restart required)" => "优先使用 X11/XWayland 实现合成器模糊（需要重启）",
        "Backend" => "后端",
        "Backend changes take effect after restarting Pretty." => "后端更改需重启 Pretty 后生效。",
        "Vertical sync (FIFO)" => "垂直同步（FIFO）",
        "GPU" => "GPU",
        "Driver" => "驱动",
        "wgpu backend" => "wgpu 后端",
        "Surface" => "窗口表面",
        "Max texture" => "最大纹理",
        "Kitty keyboard flags" => "Kitty 键盘标志",
        "Terminal modes" => "终端模式",
        "Font" => "字体",
        "Vulkan and OpenGL ES 3.0+ are implemented. Metal and Direct3D are reserved interface stubs and are never silently selected." => "已实现 Vulkan 与 OpenGL ES 3.0+；Metal 和 Direct3D 仅为保留接口，绝不会被静默选中。",
        "Kitty keyboard protocol" => "Kitty 键盘协议",
        "Negotiated flags" => "协商标志",
        "bit 1: disambiguate escape codes" => "bit 1：消除转义码歧义",
        "bit 2: report event types (repeat/release)" => "bit 2：报告事件类型（重复/释放）",
        "bit 3: report alternate keys" => "bit 3：报告备用按键",
        "bit 4: report all keys as escape codes" => "bit 4：所有按键使用转义码",
        "bit 5: report associated text" => "bit 5：报告关联文本",
        "Pretty shortcuts" => "Pretty 快捷键",
        "Open/close this settings panel" => "打开/关闭此设置面板",
        "Split focused pane to the right" => "向右拆分当前窗格",
        "Split focused pane down" => "向下拆分当前窗格",
        "Close focused pane" => "关闭当前窗格",
        "Focus neighbouring pane" => "聚焦相邻窗格",
        "Copy selection / paste" => "复制选区/粘贴",
        "Scroll history by page" => "按页回滚历史",
        "Scroll history by line" => "按行回滚历史",
        "Increase / decrease / reset font size" => "增大/减小/重置字号",
        "Force local selection over mouse reporting" => "强制本地选择（覆盖鼠标上报）",
        "Quit Pretty" => "退出 Pretty",
        "The terminal pass-through uses kitty CSI-u encoding when an application enables the protocol; otherwise it emits xterm-compatible sequences." => "应用程序启用协议时使用 kitty CSI-u 编码，否则发送 xterm 兼容序列。",
        "A hardware-accelerated terminal emulator with kitty protocol support, Vulkan and GLES backends, tiling panes and compositor blur." => "支持 kitty 协议、Vulkan/GLES 后端、平铺窗格与合成器模糊的硬件加速终端。",
        "Config" => "配置",
        "License" => "许可证",
        "Documentation" => "文档",
        "Copy" => "复制",
        "Paste" => "粘贴",
        "Select all" => "全选",
        "Clear screen" => "清屏",
        "Split right" => "向右拆分",
        "Split down" => "向下拆分",
        "Close pane" => "关闭窗格",
        "Open settings" => "打开设置",
        "Quit" => "退出",
        "Drop file to insert its path" => "拖入文件以插入路径",
        "Version" => "版本",
        "Mode" => "模式",
        "Flags" => "标志",
        "The requested backend differs from the running backend; restart required." => "请求的后端与当前运行后端不同；需要重启。",
        "Language applies immediately." => "语言会立即生效。",
        "Terminal" => "终端",
        "Close" => "关闭",
        _ => key,
    }
}
