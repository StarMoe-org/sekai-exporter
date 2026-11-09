// Unpacks level 0 of crunch files to DXT blocks with the reference decoder.
//
// Build with the include directory of Unity-Technologies/crunch (branch `unity`):
//     g++ -O2 -std=c++14 -fpermissive -w -I<crunch>/inc crunch_decode.cpp -o crunch_decode
//
// Request: little-endian u32 length, then the file. Reply: u32 length (0 on failure), then the
// blocks.
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <vector>
#ifdef __MINGW32__
#include <malloc.h>
#define malloc_usable_size _msize
#endif
#include "crn_decomp.h"
#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#endif

int main() {
#ifdef _WIN32
    _setmode(0, _O_BINARY);
    _setmode(1, _O_BINARY);
#endif
    uint32_t length;
    std::vector<uint8_t> file, blocks;
    while (fread(&length, 4, 1, stdin) == 1) {
        file.resize(length);
        if (length && fread(file.data(), 1, length, stdin) != length) return 2;
        crnd::crn_texture_info info;
        info.m_struct_size = sizeof(info);
        uint32_t size = 0;
        if (crnd::crnd_get_texture_info(file.data(), length, &info)) {
            crnd::crnd_unpack_context context = crnd::crnd_unpack_begin(file.data(), length);
            if (context) {
                uint32_t columns = (info.m_width + 3) >> 2, rows = (info.m_height + 3) >> 2;
                uint32_t pitch = columns * info.m_bytes_per_block;
                blocks.resize(pitch * rows);
                void* faces[1] = {blocks.data()};
                if (info.m_faces == 1 && crnd::crnd_unpack_level(context, faces, blocks.size(), pitch, 0)) {
                    size = blocks.size();
                }
                crnd::crnd_unpack_end(context);
            }
        }
        fwrite(&size, 4, 1, stdout);
        fwrite(blocks.data(), 1, size, stdout);
        fflush(stdout);
    }
    return 0;
}
