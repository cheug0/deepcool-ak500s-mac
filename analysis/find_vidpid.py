import re, struct, collections, math

print("### 1. heap-number ints in index.jsc within 0x0100..0xFFFF (VID/PID candidates) ###")
data = open(r'E:\ak500s_mac\analysis\asar_extracted\out\main\index.jsc','rb').read()
cnt = collections.Counter()
for off in range(0, len(data)-8, 2):
    d = struct.unpack_from('<d', data, off)[0]
    if math.isnan(d) or math.isinf(d) or d != d:
        continue
    try:
        iv = int(d)
    except (ValueError, OverflowError):
        continue
    if d == iv and 0x0100 <= iv <= 0xFFFF:
        cnt[iv] += 1
for v, c in sorted(cnt.items(), key=lambda x:-x[1])[:40]:
    print(f'  0x{v:04X} ({v}) x{c}')

print("\n### 2. u16 pair candidates in deep_service.exe ###")
svc = open(r'E:\test_exe\DeepCool\resources\service\x64\deep_service.exe','rb').read()
hits = collections.Counter()
for off in range(0, len(svc)-2, 2):
    a, b = struct.unpack_from('<HH', svc, off)
    if 0x1000 <= a <= 0xFFFF and 0x0100 <= b <= 0xFFFF:
        hits[(a,b)] += 1
for (a,b), c in sorted(hits.items(), key=lambda x:-x[1])[:25]:
    print(f'  VID 0x{a:04X} PID 0x{b:04X} x{c}')
