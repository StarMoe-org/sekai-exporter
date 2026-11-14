# compare-texture

[English](README.md)

将 `sekai-codec` 的贴图解码器与参考解码器相比对。脚本通过该 crate 的 `decode_stream` 示例程序调用解码器。

```bash
cargo build --release -p sekai-codec --features texture --example decode_stream
pip install UnityPy==1.25.4
```

ETC、EAC 与 PVRTC 与格式作者的解码器相比对：[ETCPACK](https://github.com/Ericsson/ETCPACK)（Ericsson）的 `etcdec.cxx`，以及 [PowerVR SDK](https://github.com/powervr-graphics/Native_SDK)（Imagination）的 `PVRTDecompress.cpp`。`reference/block_decode.cpp` 同时调用两者，用各自的检出构建：

```bash
g++ -O2 -std=c++17 -w -I<PowerVR SDK>/framework/PVRCore/texture reference/block_decode.cpp <ETCPACK>/source/etcdec.cxx <PowerVR SDK>/framework/PVRCore/texture/PVRTDecompress.cpp -o block_decode
```

| 脚本 | 作用 |
|---|---|
| `fuzz_astc.py <decode_stream> [块数]` | 用解码器与 ARM astcenc 分别解码各块尺寸的随机块。像素必须相等；解码器拒绝的块必须是 astcenc 解为错误色的块。 |
| `fuzz_astc_hdr.py <decode_stream> [块数] [图像...]` | 用解码器与 astcenc 的 HDR 配置解码每种块尺寸的随机块，比对 binary16 值；可另传图像文件一并比对 |
| `compare_crunch.py <decode_stream> <crunch_decode> <资源包目录>...` | 将每张不同的 crunch DXT5 贴图与 Unity crunch 库解出的块比对，块按 S3TC 公式以精确算术解码。`crunch_decode` 的构建方法见 `reference/crunch_decode.cpp` |
| `make_compressed_fixtures.py <crunch 编码器> <crunch_decode> <输出目录>` | 写出解码器测试用的合成 crunch 与 ASTC HDR 样例 |
| `fuzz_blocks.py <decode_stream> <block_decode> [图像数]` | 用解码器与参考分别解码随机尺寸的随机 ETC1、ETC2、EAC R11 与 PVRTC 图像。像素必须相等；解码器拒绝的 ETC1 图像必须含有颜色越界的差分块。 |
| `make_block_fixtures.py <block_decode> <输出目录>` | 写出解码器测试用的固定种子随机 ETC、EAC 与 PVRTC 样例及其清单 `blocks.txt` |
| `compare.py <decode_stream> <block_decode> <资源包目录>...` | 按格式统计资源包中的贴图，并将每张不重复的图与参考相比对。存在差异或被拒绝的图时，退出状态为 1。 |
| `make_fixtures.py <block_decode> <资源包目录> <输出目录>` | 由 [unity-fixtures](../unity-fixtures/README.zh-CN.md) 中 `BuildTextureFixtures.Build` 构建的资源包生成 `crates/sekai-codec/tests/fixtures/textures` 中的样例。 |

ASTC 的参考为 astcenc，ETC、EAC 与 PVRTC 的参考为经 `block_decode` 调用的 ETCPACK 与 PowerVR SDK。EAC R11 解为 11 位值，2047 表示 1.0，红色取最接近的 8 位值。PVRTC 的 RGB 格式基础格式为 RGB，alpha 为 255。未压缩格式与其字节的直接展开相比对。Alpha8 展开为黑色加存储的 alpha，即 Unity 从该格式读出的 32 位像素。

持续集成运行 `fuzz_astc.py` 与 `fuzz_astc_hdr.py`，样例由测试套件检查。
