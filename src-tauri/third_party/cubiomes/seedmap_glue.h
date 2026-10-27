/* 种子地图用的薄胶水层（Rust <-> cubiomes）。
 *
 * 目的：让 Rust 侧完全不依赖 cubiomes 的 struct 布局（Generator 里嵌了
 * LayerStack / BiomeNoise / PerlinNoise 等大量内部结构，在 Rust 里复刻一遍
 * 既啰嗦又容易跟 C 头文件脱节）。所有函数只收发 int / 指针，
 * 生成器句柄由 calloc 分配，调用方负责 seedmap_gen_free()。
 *
 * 线程约定：一个句柄只能在同一线程内构造和使用；不要在多个线程间共享。
 */
#ifndef SEEDMAP_GLUE_H_
#define SEEDMAP_GLUE_H_

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* 按 C 枚举名查版本号（"MC_1_21_WD" -> 28），未知返回 -1。
 * 只用于单测交叉校验 Rust 侧的版本表。 */
int seedmap_mc_enum(const char *name);

/* 群系 id -> 1.13+ 资源名（如 1 -> "plains"），未知返回 NULL。 */
const char *seedmap_biome_name(int mc, int id);

/* 256 个群系的地图配色（RGB 三元组，共 768 字节）。 */
void seedmap_biome_colors(unsigned char *rgb);

/* 新建/释放生成器句柄（内部 setupGenerator + applySeed）。失败返回 NULL。 */
void *seedmap_gen_new(int mc, int dim, uint64_t seed);
void seedmap_gen_free(void *gen);

/* 生成缩放比例 scale 下 (x,z) 起 w*h 个格子，x/z 单位是 scale 个方块。
 * y 是垂直位置（scale 1 时单位是方块，其余按 1:4），地表取 60 方块 => y=15。
 * 输出按 out[z*w + x] 排列。返回 0 成功。 */
int seedmap_biomes(void *gen, int scale, int x, int z, int w, int h, int y, int *out);

/* 精确到方块的群系查询。 */
int seedmap_biome_at(void *gen, int x, int z);

/* 主世界出生点（getSpawn）。 */
int seedmap_spawn(int mc, uint64_t seed, int *px, int *pz);

/* 结构配置：成功返回 1，regionSize/chunkRange 单位是区块。 */
int seedmap_struct_config(int stype, int mc, int *regionSize, int *chunkRange, int *dim);

/* 区域内结构尝试点（方块坐标），失败返回 0。 */
int seedmap_struct_pos(int stype, int mc, uint64_t seed, int regX, int regZ, int *px, int *pz);

/* 该位置是否真的能生成该结构（群系校验），返回非零表示可以。 */
int seedmap_struct_viable(void *gen, int stype, int bx, int bz);

/* 要塞：从第一个开始向外迭代，只收集落在 (cx,cz)+-radius 方块内的，
 * out 需要 2*cap 个 int。返回写入的个数。 */
int seedmap_strongholds(int mc, uint64_t seed, int cx, int cz, int radius,
        int cap, int *out);

#ifdef __cplusplus
}
#endif

#endif /* SEEDMAP_GLUE_H_ */
