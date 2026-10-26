"""Grava a saída de um programa num pty de 80x24: gravar.py SAIDA CWD 'passos' -- programa args
passos: lista JSON de [segundos, bytes] (bytes como string com escapes)."""
import os, pty, sys, time, json, fcntl, termios, struct, select, signal
out, cwd, steps = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
argv = sys.argv[sys.argv.index('--') + 1:]
pid, fd = pty.fork()
if pid == 0:
    os.chdir(cwd)
    env = {'TERM': 'xterm-256color', 'HOME': os.environ['FAKEHOME'], 'LANG': 'C.UTF-8',
           'PATH': '/usr/bin:/bin', 'COLUMNS': '80', 'LINES': '24'}
    os.execvpe(argv[0], argv, env)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
data = bytearray()
def drain(t):
    end = time.time() + t
    while True:
        left = end - time.time()
        if left <= 0: return
        r, _, _ = select.select([fd], [], [], left)
        if r:
            try: chunk = os.read(fd, 65536)
            except OSError: return
            if not chunk: return
            data.extend(chunk)
for wait, keys in steps:
    drain(wait)
    if keys:
        os.write(fd, keys.encode())
drain(0.5)
try: os.kill(pid, signal.SIGKILL)
except ProcessLookupError: pass
open(out, 'wb').write(bytes(data))
print(out, len(data))
