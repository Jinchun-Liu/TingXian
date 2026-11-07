"""Mac P0 worker sandbox. No network; exact code reads and per-task output writes."""
import json
from pathlib import Path
import subprocess
import sys

def profile(read_files=(),read_dirs=(),write_dirs=()):
    q=lambda p:json.dumps(str(Path(p).resolve()),ensure_ascii=False)
    lines=['(version 1)','(deny default)','(allow process-exec)','(allow process-fork)','(allow sysctl-read)','(allow file-read-metadata)','(allow file-read* (literal "/"))']
    # Python and macOS runtime only; the user's home is not part of this set.
    dirs={Path(sys.base_prefix),Path(sys.prefix),Path('/System/Library'),Path('/usr/lib'),Path('/usr/share/locale')}
    dirs.update(Path(p) for p in read_dirs)
    lines += ['(allow file-read* (subpath '+q(p)+'))' for p in sorted(dirs)]
    lines += ['(allow file-read* (literal '+q(p)+'))' for p in read_files]
    lines += ['(allow file-read* file-write* (subpath '+q(p)+'))' for p in write_dirs]
    lines += ['(allow file-read* file-write* (literal "/dev/null"))','(allow file-read* (literal "/dev/urandom"))']
    return '\n'.join(lines)+'\n'

def execute(command,profile_path,cwd,timeout=30):
    if sys.platform!='darwin': raise RuntimeError('Only the Mac sandbox is implemented; other platforms fail closed')
    return subprocess.run(['/usr/bin/sandbox-exec','-f',str(profile_path),*map(str,command)],cwd=cwd,env={'PATH':'/usr/bin:/bin','PYTHONDONTWRITEBYTECODE':'1','PYTHONNOUSERSITE':'1'},capture_output=True,text=True,encoding='utf-8',timeout=timeout)
