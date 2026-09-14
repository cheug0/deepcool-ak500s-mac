import re, struct
data = open(r'E:\ak500s_mac\analysis\asar_extracted\out\main\index.jsc','rb').read()
# find device-name strings and scan +-2KB for aligned u32 in plausible pid range
targets = [b'AK500S', b'Ak500s', b'AK400', b'AK620']
for t in targets:
    for m in re.finditer(re.escape(t), data):
        s, e = max(0, m.start()-2048), min(len(data), m.end()+2048)
        found = {}
        for off in range(s, e-4, 1):
            v = struct.unpack_from('<I', data, off)[0]
            if 0x0100 <= v <= 0xFFFF and data[off+3] in (0,):
                found.setdefault(v, 0)
                found[v] += 1
        cands = sorted(found.items(), key=lambda x:-x[1])[:12]
        print(f'{t.decode()} @ {m.start()}: {[(hex(v),c) for v,c in cands]}')
