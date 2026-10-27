/* 本工程新增的 MSVC 兼容垫片（不属于上游源码，上游 CMakeLists 明确只支持
 * MinGW / UCRT64 / Clang / GCC）。
 *
 * 目前只需要补一个符号：cubiomes 里用了 GCC/Clang 内建的 __builtin_popcountll，
 * MSVC 没有这个内建，会把调用当成外部函数引用，交给这里实现。
 * 用查表式的位运算实现（不依赖 POPCNT 指令，任何 x86_64 CPU 都能跑）。
 */
#if defined(_MSC_VER)

int __builtin_popcountll(unsigned long long value)
{
    value = value - ((value >> 1) & 0x5555555555555555ULL);
    value = (value & 0x3333333333333333ULL) + ((value >> 2) & 0x3333333333333333ULL);
    value = (value + (value >> 4)) & 0x0F0F0F0F0F0F0F0FULL;
    return (int) ((value * 0x0101010101010101ULL) >> 56);
}

#endif /* _MSC_VER */
