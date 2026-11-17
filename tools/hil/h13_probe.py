#!/usr/bin/env python3
"""H13: read the rkisp driver's timestamp flags/clocks for MIPI frames.

The mainpath node is multiplanar; this kernel accepts sizeof(v4l2_format)=208.
Rather than hard-coding the vendor struct layout, take G_FMT's answer and mutate
only width/height/pixelformat/field before S_FMT.
"""
import ctypes
import fcntl
import mmap
import os
import struct
import sys
import time

CAPTURE_MPLANE = 9
MEMORY_MMAP = 1
NV12 = 0x3231564E
FORMAT_SIZE = 208


def ioc(direction, nr, size):
    return (direction << 30) | (size << 16) | (ord("V") << 8) | nr


G_FMT = ioc(3, 4, FORMAT_SIZE)  # _IOWR, not _IOR (diag proof)
S_FMT = ioc(3, 5, FORMAT_SIZE)
REQBUFS = ioc(3, 8, 20)
QUERYBUF = ioc(3, 9, 88)
QBUF = ioc(3, 15, 88)
DQBUF = ioc(3, 17, 88)
STREAMON = ioc(1, 18, 4)  # _IOW(..., int)
STREAMOFF = ioc(1, 19, 4)

FLAG_TS_MASK = 0x0000E000
FLAG_TS_MONOTONIC = 0x00002000
FLAG_TS_UNKNOWN = 0x00000000
FLAG_SRC_MASK = 0x00010000
FLAG_SRC_SOE = 0x00010000


class Plane(ctypes.Structure):
    _fields_ = [
        ("bytesused", ctypes.c_uint32),
        ("length", ctypes.c_uint32),
        ("m", ctypes.c_uint64),
        ("data_offset", ctypes.c_uint32),
        ("reserved", ctypes.c_uint32 * 11),
    ]


class Buffer(ctypes.Structure):
    _fields_ = [
        ("index", ctypes.c_uint32),
        ("type", ctypes.c_uint32),
        ("bytesused", ctypes.c_uint32),
        ("flags", ctypes.c_uint32),
        ("field", ctypes.c_uint32),
        ("timestamp_sec", ctypes.c_long),
        ("timestamp_usec", ctypes.c_long),
        ("timecode", ctypes.c_uint8 * 16),
        ("sequence", ctypes.c_uint32),
        ("memory", ctypes.c_uint32),
        ("m", ctypes.c_uint64),
        ("length", ctypes.c_uint32),
        ("reserved2", ctypes.c_uint32),
        ("request_fd", ctypes.c_int32),
    ]


assert ctypes.sizeof(Buffer) == 88, ctypes.sizeof(Buffer)


def dump(tag, fmt):
    words = struct.unpack_from("=20I", fmt, 0)
    print(tag, [hex(w) for w in words])


def main():
    dev = sys.argv[1] if len(sys.argv) > 1 else "/dev/video0"
    w = int(sys.argv[2]) if len(sys.argv) > 2 else 1920
    h = int(sys.argv[3]) if len(sys.argv) > 3 else 1080
    n = int(sys.argv[4]) if len(sys.argv) > 4 else 8

    fd = os.open(dev, os.O_RDWR | os.O_NONBLOCK)
    cap = bytearray(104)
    fcntl.ioctl(fd, ioc(2, 0, 104), cap)
    fmt = bytearray(FORMAT_SIZE)
    struct.pack_into("=I", fmt, 0, CAPTURE_MPLANE)
    last = None
    for attempt in range(5):
        try:
            fcntl.ioctl(fd, G_FMT, fmt)
            break
        except OSError as e:
            last = e
            time.sleep(0.05)
    else:
        raise last
    dump("G_FMT:", fmt)
    if w and h:
        struct.pack_into("=IIII", fmt, 8, w, h, NV12, 1)  # FIELD_NONE
        fcntl.ioctl(fd, S_FMT, fmt)
        fcntl.ioctl(fd, G_FMT, fmt)
        dump("S_FMT:", fmt)
    got_w, got_h, fourcc, field = struct.unpack_from("=IIII", fmt, 8)
    sizeimage, stride = struct.unpack_from("=II", fmt, 28)
    print(f"negotiated: {got_w}x{got_h} fourcc=0x{fourcc:08x} field={field} "
          f"stride={stride} sizeimage={sizeimage}")

    rb = bytearray(20)
    struct.pack_into("=III", rb, 0, 4, CAPTURE_MPLANE, MEMORY_MMAP)
    fcntl.ioctl(fd, REQBUFS, rb)
    count = struct.unpack_from("=I", rb, 0)[0]
    print("buffers:", count)

    maps = []
    planes = []
    for i in range(count):
        p = (Plane * 1)()
        b = Buffer()
        b.index = i
        b.type = CAPTURE_MPLANE
        b.memory = MEMORY_MMAP
        b.length = 1
        b.m = ctypes.addressof(p)
        fcntl.ioctl(fd, QUERYBUF, b)
        length = p[0].length
        offset = p[0].m & 0xFFFFFFFF
        maps.append(mmap.mmap(fd, length, mmap.MAP_SHARED,
                              mmap.PROT_READ | mmap.PROT_WRITE, offset=offset))
        planes.append(p)
        fcntl.ioctl(fd, QBUF, b)

    fcntl.ioctl(fd, STREAMON, bytearray(struct.pack("=I", CAPTURE_MPLANE)))

    for _ in range(n):
        p = (Plane * 1)()
        b = Buffer()
        b.type = CAPTURE_MPLANE
        b.memory = MEMORY_MMAP
        b.length = 1
        b.m = ctypes.addressof(p)
        while True:
            try:
                fcntl.ioctl(fd, DQBUF, b)
                break
            except OSError as e:
                if e.errno != 11:
                    raise
                time.sleep(0.002)
        mono = time.clock_gettime(time.CLOCK_MONOTONIC)
        ts_ns = b.timestamp_sec * 1_000_000_000 + b.timestamp_usec * 1000
        mono_ns = int(mono * 1_000_000_000)
        ts_kind = {
            FLAG_TS_MONOTONIC: "MONOTONIC",
            FLAG_TS_UNKNOWN: "UNKNOWN",
        }.get(b.flags & FLAG_TS_MASK, f"OTHER(0x{b.flags & FLAG_TS_MASK:04x})")
        src = "SOE(SOF)" if (b.flags & FLAG_SRC_MASK) == FLAG_SRC_SOE else "EOF"
        print(f"seq={b.sequence:5d} used={p[0].bytesused:8d} "
              f"flags=0x{b.flags:08x} tstype={ts_kind} src={src} "
              f"delta_ms={(mono_ns - ts_ns) / 1e6:7.2f}")
        fcntl.ioctl(fd, QBUF, b)

    fcntl.ioctl(fd, STREAMOFF, bytearray(struct.pack("=I", CAPTURE_MPLANE)))
    for m in maps:
        m.close()
    os.close(fd)


if __name__ == "__main__":
    main()
