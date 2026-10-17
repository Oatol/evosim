#!/usr/bin/env python3
"""从系统字体里裁出界面用到的那一小撮汉字，嵌进 exe。

为什么不用整字体：微软雅黑 19 MB，黑体 9.7 MB——直接塞进 exe 太重。
而界面上的中文是**有限的**（按钮、参数名、细胞简介、提示语），扫一遍源码
就能把它们全找出来，裁出来的子集只有几百 KB。

用法（在 rust/ 目录下）：
    python tools/subset-font.py            # 生成子集
    python tools/subset-font.py --check    # 只检查：源码里有没有子集覆盖不到的字

**改过 UI 文案之后要重跑一次**，否则新加的字会显示成方块。
（`--check` 就是为这件事准备的：CI 或提交前跑一下。）

产物：
    crates/app/assets/ui-cjk-subset.ttf   嵌进 exe 的字体
    crates/app/assets/ui-chars.txt        覆盖到的字符清单（Rust 测试用它做断言）
"""

import os
import sys
import glob

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)  # rust/
SRC_GLOBS = [
    os.path.join(ROOT, "crates", "*", "src", "**", "*.rs"),
    os.path.join(ROOT, "crates", "*", "tests", "**", "*.rs"),
    os.path.join(ROOT, "crates", "*", "src", "*.wgsl"),
]
OUT_FONT = os.path.join(ROOT, "crates", "app", "assets", "ui-cjk-subset.ttf")
OUT_CHARS = os.path.join(ROOT, "crates", "app", "assets", "ui-chars.txt")

# 系统字体候选（TTC 取 index 0 = 微软雅黑 Regular）
FONT_CANDIDATES = [
    ("C:/Windows/Fonts/msyh.ttc", 0),
    ("C:/Windows/Fonts/msyh.ttf", 0),
    ("C:/Windows/Fonts/simhei.ttf", 0),
]

# 一定会用到的 ASCII 与常用符号（数字、单位、标点）
ALWAYS = set(chr(c) for c in range(0x20, 0x7F))
ALWAYS |= set("·×—…「」『』“”‘’《》、，。！？：；（）【】％±°→←↑↓●○■□◆◇★☆")


def collect_chars():
    """扫源码，收集所有非 ASCII 字符（注释里的也算——多几个字形不碍事）"""
    chars = set(ALWAYS)
    files = []
    for g in SRC_GLOBS:
        files.extend(glob.glob(g, recursive=True))
    for path in sorted(set(files)):
        with open(path, encoding="utf-8") as f:
            for ch in f.read():
                if ord(ch) > 0x7F:
                    chars.add(ch)
    return chars, files


def pick_font():
    for path, index in FONT_CANDIDATES:
        if os.path.exists(path):
            return path, index
    return None, 0


def main():
    chars, files = collect_chars()
    print("扫描 %d 个源文件，需要 %d 个字符" % (len(files), len(chars)))

    font_path, index = pick_font()
    if font_path is None:
        print("找不到系统 CJK 字体（msyh.ttc / simhei.ttf），无法生成子集")
        return 1

    if "--check" in sys.argv:
        from fontTools.ttLib import TTFont

        f = TTFont(OUT_FONT, fontNumber=0) if OUT_FONT.endswith(".ttc") else TTFont(OUT_FONT)
        cmap = f.getBestCmap()
        missing = sorted(ch for ch in chars if ord(ch) not in cmap)
        if missing:
            print("子集缺 %d 个字：%s" % (len(missing), "".join(missing[:80])))
            print("请重跑 python tools/subset-font.py")
            return 1
        print("子集覆盖完整（%d 个字符）" % len(cmap))
        return 0

    from fontTools import subset

    os.makedirs(os.path.dirname(OUT_FONT), exist_ok=True)
    unicodes = sorted(ord(c) for c in chars)

    opts = subset.Options()
    opts.font_number = index
    opts.layout_features = ["*"]      # 保留全部 OpenType 特性（避免某些字间距/替换丢字）
    opts.name_IDs = ["*"]
    opts.notdef_outline = True
    opts.recalc_bounds = True
    opts.drop_tables += ["DSIG", "hdmx", "VDMX", "LTSH", "gasp"]
    # 关掉 hinting：屏幕上我们是 12px 直接画，egui 自己按像素网格对齐，
    # 字体自带的 TrueType 指令用不上，留着只是白占体积（约省一半）
    opts.hinting = False

    font = subset.load_font(font_path, opts)
    subsetter = subset.Subsetter(options=opts)
    subsetter.populate(unicodes=unicodes)
    subsetter.subset(font)
    subset.save_font(font, OUT_FONT, opts)

    with open(OUT_CHARS, "w", encoding="utf-8") as f:
        f.write("".join(sorted(chars)))

    size = os.path.getsize(OUT_FONT)
    print("源字体 %s（%.1f MB）" % (font_path, os.path.getsize(font_path) / 1048576))
    print("子集   %s（%d KB，%d 字符）" % (OUT_FONT, size // 1024, len(chars)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
