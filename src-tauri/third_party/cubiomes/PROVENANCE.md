# cubiomes（第三方 C 源码）

来源: 用户提供的上游 master 快照 `D:\minecraft\cubiomes-master`（只读素材，本工程未改动它）
快照内容: 顶层 biomenoise/biomes/carver/finders/generator/layers/noise/terrainnoise/util/xradv、
        features/（结构拼块与矿脉）、loot/（战利品表）、tables/（1.18+ 群系查询表 8 份）、
        xrms.h、docs/、tests/、CMakeLists.txt 等
commit: 快照未附带 .git，读不到 commit 信息，故无法给出 hash
版本线索: biomes.h 的 MCVersion 枚举覆盖到 MC_1_21_11 与 MC_26_1 / MC_26_2 / MC_26_3
        （MC_NEWEST = MC_26_3），README 提到 "abandoned camp generation (26.3+)"
快照日期: 2026-10-27（本机 vendor 日期）
许可: MIT（保留 LICENSE 原文）
取舍: 不用用户另一份 `D:\minecraft\cubiomes-21.11.18`——那是 2021 年老版本，layers.h
      的 MCVersion 只到 MC_1_17，且是旧的 LayerStack API，无法支持 1.21.x
改动:
  1) 新增 seedmap_glue.c / seedmap_glue.h（薄胶水层，Rust 侧不碰任何 C 结构体布局）
  2) 新增 seedmap_compat.c（MSVC 垫片：补 __builtin_popcountll）
  3) 对上游源码打了 5 处 MSVC 兼容补丁（上游 CMakeLists.txt 明确写着
     "MSVC is not supported for this project"，而本工程是 Windows + MSVC 工具链）。
     补丁只处理「MSVC 不支持 C99 变长数组 / static 初始化里的复合字面量」，
     不改变任何算法与数值：

| 文件 | 位置 | 原因 | 处理 |
| --- | --- | --- | --- |
| terrainnoise.c | generateRegion() | uint8_t (*blocks2)[height]、double (*ds)[n] 变长数组类型 | 改连续缓冲 + 手动跨步（语义等价） |
| finders.c | isViableEndCityTerrain() | double ncol[nx][nz][yn] 三维变长数组 | malloc 连续缓冲 + NCOL(i,j) 下标，按出口 free |
| finders.c | isViableNetherFossilTerrain() | const int cellHeight/colYMin/colYMax 当数组长度 | 改成 enum 常量（数值不变） |
| finders.c | Shipwreck 拼块表 | static 初始化里用复合字面量 (Pos){4,9} | 去掉 static，改自动存储 |
| features/abandoned_camp.c | camp 拼块洗牌 | int positions[campCount]，campCount 是 const int | 改 enum 常量（3*15+4 不变） |

## 编译方式

由 src-tauri/build.rs 用 cc crate 编译下面 15 个 .c（全部静态链进 exe，运行时
不需要任何外部 exe/DLL）：

```
noise.c  biomes.c  layers.c  biomenoise.c  terrainnoise.c  generator.c  finders.c  util.c
features/abandoned_camp.c  features/end_city.c  features/fortress.c
features/mineshaft.c  features/stronghold.c
seedmap_compat.c  seedmap_glue.c
```

没有编进来的上游文件：quadbase.c（命令行工具，需要 Windows 线程与文件系统）、
tests.c（自带测试框架）、xradv.c（独立实验 API，没人调用）、carver.c 与
features/ore.c（util.c 只用到它们头文件里的枚举名，没有引用实现——链接器可以证明：
拿掉后没有未解析符号）、loot/*.c（战利品表，本功能不用）。

第三方源码关掉警告（build.rs 里 warnings(false)），并把 -fwrapv 传给支持的编译器
（cubiomes 的有符号整数回绕是有意的，属实现语义而非 UB 假设；MSVC 默认就是回绕，跳过该 flag）。
