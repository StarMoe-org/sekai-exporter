"""Build the HCA test fixtures from generated audio.

Usage:
    python make_fixtures.py <VGAudioCli> <vgmstream-cli> <output directory>

Generates a sine tone, a stereo tone and silence, and encodes them to HCA with VGAudio. Encrypted
variants are made here from the stereo tone, using a key that exists only for these tests, and a
version 3 stream is made of random frames. The expected samples of each file come from vgmstream
and are written to `expected.txt` as `file rate channels bytes sha256 key subkey`.
"""

import hashlib
import math
import pathlib
import random
import struct
import subprocess
import sys
import tempfile
import wave

from compare import wav_samples

RATE = 44100
LENGTH = RATE // 2
TEST_KEY = 0x0123456789ABCDE
TEST_SUBKEY = 0x4A1C


def crc16(data):
    value = 0
    for byte in data:
        value ^= byte << 8
        for _ in range(8):
            value = ((value << 1) ^ 0x8005 if value & 0x8000 else value << 1) & 0xFFFF
    return value


def write_wav(path, channels, sample):
    frames = bytearray()
    for i in range(LENGTH):
        for channel in range(channels):
            frames += struct.pack("<h", sample(i, channel))
    with wave.open(str(path), "wb") as out:
        out.setnchannels(channels)
        out.setsampwidth(2)
        out.setframerate(RATE)
        out.writeframes(bytes(frames))


def tone(frequency):
    return lambda i, channel: int(12000 * math.sin(2 * math.pi * (frequency + 220 * channel) * i / RATE))


def nibbles(key):
    multiplier = ((key & 1) << 3) | 5
    increment = (key & 0xE) | 1
    value = key >> 4
    out = []
    for _ in range(16):
        value = (value * multiplier + increment) & 0xF
        out.append(value)
    return out


def cipher_table(key, subkey):
    if subkey:
        key = key * ((subkey << 16) | ((~subkey & 0xFFFF) + 2)) & 0xFFFFFFFFFFFFFFFF
    k = (key - 1).to_bytes(8, "little")
    seed = [
        k[1], k[1] ^ k[6], k[2] ^ k[3], k[2], k[2] ^ k[1], k[3] ^ k[4], k[3], k[3] ^ k[2],
        k[4] ^ k[5], k[4], k[4] ^ k[3], k[5] ^ k[6], k[5], k[5] ^ k[4], k[6] ^ k[1], k[6],
    ]
    rows = nibbles(k[0])
    base = [(rows[r] << 4) | c for r in range(16) for c in nibbles(seed[r])]
    table = [0] * 256
    position, x = 1, 0
    for _ in range(256):
        x = (x + 17) & 0xFF
        if base[x] not in (0, 0xFF):
            table[position] = base[x]
            position += 1
    table[255] = 0xFF
    return table


def stream_layout(stream):
    size = struct.unpack(">H", stream[6:8])[0]
    header = bytes(b & 0x7F for b in stream[:size])
    comp = header.index(b"comp")
    frame_size = struct.unpack(">H", stream[comp + 4 : comp + 6])[0]
    return size, frame_size


def encrypt(stream, key, subkey):
    """Returns a copy of a stream of cipher type 0 turned into cipher type 56."""
    size, frame_size = stream_layout(stream)
    plain = bytearray(stream)
    cipher = bytes(b & 0x7F for b in plain[:size]).index(b"ciph")
    plain[cipher + 4 : cipher + 6] = struct.pack(">H", 56)
    plain[size - 2 : size] = struct.pack(">H", crc16(plain[: size - 2]))
    table = cipher_table(key, subkey)
    inverse = [0] * 256
    for stored, value in enumerate(table):
        inverse[value] = stored
    for start in range(size, len(plain), frame_size):
        frame = bytes(inverse[b] for b in plain[start : start + frame_size - 2])
        plain[start : start + frame_size] = frame + struct.pack(">H", crc16(frame))
    return bytes(plain)


def random_version_3(seed):
    """A mono version 3 stream whose frames are random, with fixed-width scale factors."""
    generator = random.Random(seed)
    frame_size = 1024
    frame_count = 12
    fmt = b"fmt\0" + bytes([1]) + RATE.to_bytes(3, "big") + struct.pack(">IHH", frame_count, 0, 0)
    comp = b"comp" + struct.pack(">HBBBBBBBBBB", frame_size, 0, 15, 1, 1, 128, 60, 0, 7, 0, 0)
    size = 8 + len(fmt) + len(comp) + 2
    head = b"HCA\0" + struct.pack(">HH", 0x300, size) + fmt + comp
    stream = bytearray(head + struct.pack(">H", crc16(head)))
    for _ in range(frame_count):
        frame = bytearray(generator.randbytes(frame_size - 2))
        frame[:2] = b"\xff\xff"
        # The three bits after the noise level and boundary select fixed-width scale factors.
        frame[4] = (frame[4] & 0x1F) | 0xC0
        # Moderate scale factors keep the samples in range.
        for index in range(70):
            value = generator.randrange(38)
            for bit in range(6):
                position = 35 + index * 6 + bit
                mask = 0x80 >> (position % 8)
                frame[position // 8] = frame[position // 8] & ~mask | (mask if value >> (5 - bit) & 1 else 0)
        stream += frame + struct.pack(">H", crc16(frame))
    return bytes(stream)


def reference(tool, directory, stream, key, subkey):
    source = directory / ("stream.adx" if stream[:2] == b"\x80\x00" else "stream.hca")
    target = directory / "stream.wav"
    source.write_bytes(stream)
    key_file = directory / ".hcakey"
    if key:
        key_file.write_bytes(struct.pack(">Q", key) + (struct.pack(">H", subkey) if subkey else b""))
    elif key_file.exists():
        key_file.unlink()
    subprocess.run([tool, "-i", "-o", str(target), str(source)], check=True, capture_output=True)
    return wav_samples(target)


def main():
    encoder, tool, output = sys.argv[1:4]
    out = pathlib.Path(output)
    out.mkdir(parents=True, exist_ok=True)
    streams = {}
    with tempfile.TemporaryDirectory() as temporary:
        directory = pathlib.Path(temporary)
        inputs = {
            "sine_mono": (1, tone(440)),
            "silence_mono": (1, lambda i, channel: 0),
            "sine_stereo": (2, tone(330)),
        }
        for name, (channels, sample) in inputs.items():
            source = directory / f"{name}.wav"
            target = directory / f"{name}.hca"
            write_wav(source, channels, sample)
            subprocess.run([encoder, "-c", str(source), str(target), "--hcaquality", "High"], check=True, capture_output=True)
            streams[name] = (target.read_bytes(), 0, 0)
            adx = directory / f"{name}.adx"
            subprocess.run([encoder, "-c", str(source), str(adx)], check=True, capture_output=True)
            streams[f"{name}.adx"] = (adx.read_bytes(), 0, 0)
        stereo = streams["sine_stereo"][0]
        streams["sine_stereo_key"] = (encrypt(stereo, TEST_KEY, 0), TEST_KEY, 0)
        streams["sine_stereo_subkey"] = (encrypt(stereo, TEST_KEY, TEST_SUBKEY), TEST_KEY, TEST_SUBKEY)
        streams["random_v3"] = (random_version_3(7), 0, 0)

        lines = []
        for name, (stream, key, subkey) in streams.items():
            file = name if name.endswith(".adx") else f"{name}.hca"
            (out / file).write_bytes(stream)
            rate, channels, samples = reference(tool, directory, stream, key, subkey)
            digest = hashlib.sha256(samples).hexdigest()
            lines.append(f"{file} {rate} {channels} {len(samples)} {digest} {key:x} {subkey:x}")
            print(lines[-1])
    (out / "expected.txt").write_text("\n".join(lines) + "\n", newline="\n")


if __name__ == "__main__":
    main()
