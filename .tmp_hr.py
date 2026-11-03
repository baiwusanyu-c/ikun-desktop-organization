import io, sys
path = sys.argv[1]
start = int(sys.argv[2]); end = int(sys.argv[3])
L = io.open(path, encoding='utf-8', errors='replace').read().splitlines()
out = []
for i in range(start-1, min(end, len(L))):
    out.append('%d: %s' % (i+1, L[i]))
sys.stdout.buffer.write(('\n'.join(out)).encode('utf-8'))
