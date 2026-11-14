// Decodes ETC1, ETC2, EAC R11 and PVRTC 1 images with the reference decoders of their owners:
// `etcdec.cxx` of Ericsson's ETCPACK and `PVRTDecompress.cpp` of Imagination's PowerVR SDK.
//
// Build with a checkout of each:
//     g++ -O2 -std=c++17 -w -I<PowerVR SDK>/framework/PVRCore/texture block_decode.cpp
//         <ETCPACK>/source/etcdec.cxx <PowerVR SDK>/framework/PVRCore/texture/PVRTDecompress.cpp
//         -o block_decode
//
// Request: little-endian i32 Unity texture format, u32 width, u32 height, u32 length, then the
// data. Reply: u32 length (0 for a format it does not decode), then RGBA in stored row order.
//
// EAC R11 decodes to eleven bits, where 2047 stands for 1.0; red is the nearest eight-bit value.
// The PVRTC RGB formats have the base format RGB, so their alpha is 255.
#include <cstdint>
#include <cstdio>
#include <vector>
#include "PVRTDecompress.h"
#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#endif

typedef unsigned char uint8;
void setupAlphaTable();
void decompressBlockDiffFlipC(unsigned int, unsigned int, uint8*, int, int, int, int, int);
void decompressBlockETC2c(unsigned int, unsigned int, uint8*, int, int, int, int, int);
void decompressBlockAlphaC(uint8*, uint8*, int, int, int, int, int);
void decompressBlockAlpha16bitC(uint8*, uint8*, int, int, int, int, int);

static uint32_t be32(const uint8_t* p) {
    return uint32_t(p[0]) << 24 | uint32_t(p[1]) << 16 | uint32_t(p[2]) << 8 | p[3];
}

// Decodes the ETC and EAC formats into `rgba`, `width` x `height` rounded up to whole blocks.
static bool decode_etc(int32_t format, uint32_t width, uint32_t height, const uint8_t* data,
                       std::vector<uint8_t>& rgba) {
    size_t block = format == 47 ? 16 : 8;
    if (format != 34 && format != 41 && format != 45 && format != 47) return false;
    int columns = (width + 3) / 4, rows = (height + 3) / 4, w = columns * 4, h = rows * 4;
    rgba.assign(size_t(w) * h * 4, 255);
    std::vector<uint8_t> red(size_t(w) * h * 2);
    for (int row = 0; row < rows; row++) {
        for (int column = 0; column < columns; column++) {
            uint8_t* p = const_cast<uint8_t*>(data + (size_t(row) * columns + column) * block);
            int x = column * 4, y = row * 4;
            switch (format) {
            case 34: decompressBlockDiffFlipC(be32(p), be32(p + 4), rgba.data(), w, h, x, y, 4); break;
            case 45: decompressBlockETC2c(be32(p), be32(p + 4), rgba.data(), w, h, x, y, 4); break;
            case 47:
                decompressBlockAlphaC(p, rgba.data() + 3, w, h, x, y, 4);
                decompressBlockETC2c(be32(p + 8), be32(p + 12), rgba.data(), w, h, x, y, 4);
                break;
            case 41: decompressBlockAlpha16bitC(p, red.data(), w, h, x, y, 1); break;
            }
        }
    }
    if (format == 41) {
        for (size_t i = 0; i < size_t(w) * h; i++) {
            // etcdec writes (v << 5) + (v >> 6) for the eleven-bit value v, big-endian.
            uint32_t eleven = (uint32_t(red[i * 2]) << 8 | red[i * 2 + 1]) >> 5;
            rgba[i * 4] = uint8_t((eleven * 255 + 1023) / 2047);
            rgba[i * 4 + 1] = rgba[i * 4 + 2] = 0;
        }
    }
    // Keep the image's own columns.
    std::vector<uint8_t> image(size_t(width) * height * 4);
    for (uint32_t y = 0; y < height; y++)
        for (uint32_t x = 0; x < width * 4; x++) image[(size_t(y) * width) * 4 + x] = rgba[(size_t(y) * w) * 4 + x];
    rgba.swap(image);
    return true;
}

static bool decode_pvrtc(int32_t format, uint32_t width, uint32_t height, const uint8_t* data,
                         std::vector<uint8_t>& rgba) {
    if (format < 30 || format > 33) return false;
    rgba.assign(size_t(width) * height * 4, 0);
    pvr::PVRTDecompressPVRTC(data, format <= 31, width, height, rgba.data());
    if (format % 2 == 0)
        for (size_t i = 3; i < rgba.size(); i += 4) rgba[i] = 255;
    return true;
}

int main() {
#ifdef _WIN32
    _setmode(0, _O_BINARY);
    _setmode(1, _O_BINARY);
#endif
    setupAlphaTable();
    int32_t format;
    uint32_t width, height, length;
    std::vector<uint8_t> data, rgba;
    while (fread(&format, 4, 1, stdin) == 1) {
        if (fread(&width, 4, 1, stdin) != 1 || fread(&height, 4, 1, stdin) != 1 ||
            fread(&length, 4, 1, stdin) != 1)
            return 2;
        data.resize(length);
        if (length && fread(data.data(), 1, length, stdin) != length) return 2;
        bool decoded = decode_etc(format, width, height, data.data(), rgba) ||
                       decode_pvrtc(format, width, height, data.data(), rgba);
        uint32_t size = decoded ? uint32_t(rgba.size()) : 0;
        fwrite(&size, 4, 1, stdout);
        fwrite(rgba.data(), 1, size, stdout);
        fflush(stdout);
    }
    return 0;
}
