import re
data = open(r'E:\test_exe\DeepCool\resources\service\x64\DeepCoolDisplayService.exe','rb').read()
idxs = [m.start() for m in re.finditer(b'MSCF', data)]
print('MSCF found at:', idxs)
for i, off in enumerate(idxs):
    # CAB header: nextoffset u32 at +4 (after 'MSCF' res1... actually: sig(4) res1(4) cbFile(4)...)
    import struct
    cbFile = struct.unpack_from('<I', data, off+8)[0]
    print(f'cab {i}: offset={off} declared_size={cbFile}')
    if 0 < cbFile < 5_000_000:
        open(f'E:/ak500s_mac/analysis/embedded_{i}.cab','wb').write(data[off:off+cbFile])
        print(f'  -> saved embedded_{i}.cab')
