/* 见 seedmap_glue.h：把 cubiomes 的 C 接口收敛成只吃基础类型的函数。 */
#include "seedmap_glue.h"

#include <stdlib.h>
#include <string.h>

#include "finders.h"
#include "biomes.h"
#include "util.h"

/* 地表采样平面：1:4 尺度下 y=15 约等于 60 方块高（cubiomes README 的推荐值） */
#define SEEDMAP_SURFACE_Y4 15

typedef struct
{
    Generator g;
} SeedmapGen;

int seedmap_mc_enum(const char *name)
{
    static const struct { const char *name; int value; } table[] = {
        { "MC_UNDEF",  MC_UNDEF  },
        { "MC_B1_7",   MC_B1_7   },
        { "MC_B1_8",   MC_B1_8   },
        { "MC_1_0",    MC_1_0    },
        { "MC_1_1",    MC_1_1    },
        { "MC_1_2",    MC_1_2    },
        { "MC_1_3",    MC_1_3    },
        { "MC_1_4",    MC_1_4    },
        { "MC_1_5",    MC_1_5    },
        { "MC_1_6",    MC_1_6    },
        { "MC_1_7",    MC_1_7    },
        { "MC_1_8",    MC_1_8    },
        { "MC_1_9",    MC_1_9    },
        { "MC_1_10",   MC_1_10   },
        { "MC_1_11",   MC_1_11   },
        { "MC_1_12",   MC_1_12   },
        { "MC_1_13",   MC_1_13   },
        { "MC_1_14",   MC_1_14   },
        { "MC_1_15",   MC_1_15   },
        { "MC_1_16_1", MC_1_16_1 },
        { "MC_1_16",   MC_1_16   },
        { "MC_1_17",   MC_1_17   },
        { "MC_1_18",   MC_1_18   },
        { "MC_1_19_2", MC_1_19_2 },
        { "MC_1_19",   MC_1_19   },
        { "MC_1_20",   MC_1_20   },
        { "MC_1_21_1", MC_1_21_1 },
        { "MC_1_21_3", MC_1_21_3 },
        { "MC_1_21_WD", MC_1_21_WD },
        { "MC_1_21_4", MC_1_21_4 },
        { "MC_1_21_5", MC_1_21_5 },
        { "MC_1_21_6", MC_1_21_6 },
        { "MC_1_21_9", MC_1_21_9 },
        { "MC_1_21_11", MC_1_21_11 },
        { "MC_26_1", MC_26_1 },
        { "MC_26_2", MC_26_2 },
        { "MC_26_3", MC_26_3 },
        { "MC_NEWEST", MC_NEWEST },
    };
    size_t i;
    if (!name)
        return -1;
    for (i = 0; i < sizeof(table) / sizeof(table[0]); i++)
        if (strcmp(table[i].name, name) == 0)
            return table[i].value;
    return -1;
}

const char *seedmap_biome_name(int mc, int id)
{
    const char *s = biome2str(mc, id);
    return s && *s ? s : NULL;
}

void seedmap_biome_colors(unsigned char *rgb)
{
    unsigned char colors[256][3];
    int i;
    if (!rgb)
        return;
    initBiomeColors(colors);
    for (i = 0; i < 256; i++)
    {
        rgb[3*i+0] = colors[i][0];
        rgb[3*i+1] = colors[i][1];
        rgb[3*i+2] = colors[i][2];
    }
}

void *seedmap_gen_new(int mc, int dim, uint64_t seed)
{
    SeedmapGen *h = (SeedmapGen *)calloc(1, sizeof(SeedmapGen));
    if (!h)
        return NULL;
    setupGenerator(&h->g, mc, 0);
    applySeed(&h->g, dim, seed);
    return h;
}

void seedmap_gen_free(void *gen)
{
    free(gen);
}

int seedmap_biomes(void *gen, int scale, int x, int z, int w, int h, int y, int *out)
{
    SeedmapGen *g = (SeedmapGen *)gen;
    Range r;
    int *cache;
    int err, i, n;

    if (!g || !out || w <= 0 || h <= 0)
        return -1;

    r.scale = scale;
    r.x = x;
    r.z = z;
    r.sx = w;
    r.sz = h;
    r.y = y;
    r.sy = 1;

    cache = allocCache(&g->g, r);
    if (!cache)
        return -2;

    err = genBiomes(&g->g, cache, r);
    if (!err)
    {
        n = w * h;
        for (i = 0; i < n; i++)
            out[i] = cache[i];
    }
    free(cache);
    return err;
}

int seedmap_biome_at(void *gen, int x, int z)
{
    SeedmapGen *g = (SeedmapGen *)gen;
    if (!g)
        return none;
    /* 和地图一致：按 1:4 的地表平面采样（y=15 即约 60 方块高），
     * 否则 1.18+ 在 y=0 采到的是溶洞/深暗之域这类地下群系。 */
    return getBiomeAt(&g->g, 4, x >> 2, SEEDMAP_SURFACE_Y4, z >> 2);
}

int seedmap_spawn(int mc, uint64_t seed, int *px, int *pz)
{
    Generator g;
    Pos p;

    setupGenerator(&g, mc, 0);
    applySeed(&g, DIM_OVERWORLD, seed);
    p = getSpawn(&g);

    if (px) *px = p.x;
    if (pz) *pz = p.z;
    return 0;
}

int seedmap_struct_config(int stype, int mc, int *regionSize, int *chunkRange, int *dim)
{
    StructureConfig sconf;
    if (!getStructureConfig(stype, mc, &sconf))
        return 0;
    if (regionSize) *regionSize = sconf.regionSize;
    if (chunkRange) *chunkRange = sconf.chunkRange;
    if (dim)        *dim = sconf.dim;
    return 1;
}

int seedmap_struct_pos(int stype, int mc, uint64_t seed, int regX, int regZ,
        int *px, int *pz)
{
    Pos p;
    if (!getStructurePos(stype, mc, seed, regX, regZ, &p))
        return 0;
    if (px) *px = p.x;
    if (pz) *pz = p.z;
    return 1;
}

int seedmap_struct_viable(void *gen, int stype, int bx, int bz)
{
    SeedmapGen *g = (SeedmapGen *)gen;
    if (!g)
        return 0;
    return isViableStructurePos(stype, &g->g, bx, bz, 0) ? 1 : 0;
}

int seedmap_strongholds(int mc, uint64_t seed, int cx, int cz, int radius,
        int cap, int *out)
{
    Generator g;
    StrongholdIter sh;
    int n = 0;
    int give_up_dist = radius * 2 + 2048;

    if (!out || cap <= 0)
        return 0;

    setupGenerator(&g, mc, 0);
    applySeed(&g, DIM_OVERWORLD, seed);
    initFirstStronghold(&sh, mc, seed);

    while (n < cap)
    {
        int left = nextStronghold(&sh, &g);
        int dx = sh.pos.x - cx;
        int dz = sh.pos.z - cz;

        if (sh.pos.x >= cx - radius && sh.pos.x <= cx + radius &&
            sh.pos.z >= cz - radius && sh.pos.z <= cz + radius)
        {
            out[2*n+0] = sh.pos.x;
            out[2*n+1] = sh.pos.z;
            n++;
        }
        /* 要塞是向外螺旋分布的：已经远到不可能再回到搜索框里就停。 */
        if ((dx < 0 ? -dx : dx) > give_up_dist || (dz < 0 ? -dz : dz) > give_up_dist)
            break;
        if (left <= 0)
            break;
    }
    return n;
}
