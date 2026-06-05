import sys, re
t = open(sys.argv[1]).read()
m = re.search(r'blob "((?:[^"\\]|\\.)*)"', t, re.S)
if not m:
    sys.stderr.write("NOMATCH\n"); sys.exit(1)
b = m.group(1); out = bytearray(); i = 0
hexd = "0123456789abcdefABCDEF"
while i < len(b):
    if b[i] == '\\':
        n = b[i+1]
        if n in hexd and i + 2 < len(b) and b[i+2] in hexd:
            out.append(int(b[i+1:i+3], 16)); i += 3; continue
        mp = {'n': 10, 't': 9, 'r': 13, '\\': 92, '"': 34}
        out.append(mp.get(n, ord(n))); i += 2; continue
    out.append(ord(b[i])); i += 1
print(out.hex())
