"""Run `alphabet_finetune.py` on a TSUBAME free interactive GPU node.

  python scripts/ocr/tsubame_train.py --run NAME --data training.npz \
    [--replay-data synthetic.npz] [--model base.onnx] [--hours 0:30] \
    -- --replay-fraction 0.5 --augment 0.7 --epochs 10

Only the training arrays leave this machine: `training.npz` from
`matrix_video.py build` (shuffled single-cell crops and labels of the training
and validation cards, no test split, lineage, video or whole-card image) and,
optionally, synthetic print. Data files are stored remotely under their content
hash, so an unchanged file is not sent again. The job runs through
`iqrsh -l h_rt=...` without `-g` (the free interactive queue) in the remote
project's virtual environment; the exported model, `training.json` and the log
come back to `.cache/tsubame/NAME/`. Evaluation and session replay stay local.

The login nodes throttle bursts of new connections, so a run holds one SSH
connection (ControlMaster) for its upload (one rsync), the job and the
download (one rsync). The iqrsh job runs in a tmux session on the login node
(`isct-NAME`), so it survives a dropped connection; the script polls for its
exit code once a minute. On Windows it runs in WSL (`--wsl`, default
Ubuntu), whose OpenSSH supports connection sharing and ships rsync; WSL's
`~/.ssh` needs the key and the host alias.
"""

import argparse
import hashlib
import os
from pathlib import Path, PureWindowsPath
import shlex
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
SCRIPTS = ("alphabet_finetune.py", "alphabet_model.py", "alphabet_calibration.py")


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def posix(path, wsl):
    """The path as the shell running the driver sees it."""
    if not wsl:
        return str(path)
    windows = PureWindowsPath(Path(path).resolve())
    return f"/mnt/{windows.drive[0].lower()}/" + "/".join(windows.parts[1:])


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--run", required=True, help="Run name: letters, digits, - and _")
    p.add_argument("--data", type=Path, required=True)
    p.add_argument("--replay-data", type=Path)
    p.add_argument("--model", type=Path, default=ROOT / "crates/app-ocr/models/alphabet/alphabet-printed.onnx")
    p.add_argument("--host", default="tsubame")
    p.add_argument("--remote-dir", default="isct-matrix-ocr", help="Relative to the remote home")
    p.add_argument("--python", default=".venv/bin/python", help="Relative to --remote-dir")
    p.add_argument("--hours", default="0:30", help="iqrsh h_rt limit, H:MM")
    p.add_argument("--wsl", default="Ubuntu" if os.name == "nt" else "",
                   help="WSL distribution that runs ssh and rsync (empty: run them directly)")
    p.add_argument("finetune", nargs=argparse.REMAINDER, help="Arguments after -- go to alphabet_finetune.py")
    a = p.parse_args()
    if not a.run or not all(c.isalnum() or c in "-_" for c in a.run):
        p.error("--run must be letters, digits, - or _")
    extra = a.finetune[1:] if a.finetune[:1] == ["--"] else a.finetune
    if any(flag in extra for flag in ("--data", "--replay-data", "--model", "--output", "--device")):
        p.error("Data, model, output and device are set by this script")
    hours, minutes = (int(v) for v in a.hours.split(":"))

    # The stage mirrors the remote layout: data/<hash><suffix> and runs/<run>/.
    local = ROOT / ".cache/tsubame" / a.run
    stage = local / "stage"
    shutil.rmtree(stage, ignore_errors=True)
    (stage / "data").mkdir(parents=True)
    work = stage / "runs" / a.run
    work.mkdir(parents=True)
    named = {}
    for key, path in (("data", a.data), ("replay", a.replay_data), ("model", a.model)):
        if path is None:
            continue
        named[key] = f"{digest(path)[:16]}{path.suffix}"
        # A hard link keeps size and mtime, which rsync compares, without a copy.
        try:
            os.link(path, stage / "data" / named[key])
        except OSError:
            shutil.copy2(path, stage / "data" / named[key])
    for script in SCRIPTS:
        shutil.copyfile(HERE / script, work / script)
    store = f'"$HOME/{a.remote_dir}/data'
    command = [f'"$HOME/{a.remote_dir}/{a.python}"', "alphabet_finetune.py",
               "--model", f'{store}/{named["model"]}"', "--data", f'{store}/{named["data"]}"',
               "--output", "out", "--device", "cuda", *map(shlex.quote, extra)]
    if "replay" in named:
        command += ["--replay-data", f'{store}/{named["replay"]}"']
    (work / "remote.sh").write_text("\n".join([
        "#!/bin/bash", "set -uo pipefail", f'cd "$HOME/{a.remote_dir}/runs/{a.run}"',
        "nvidia-smi -L > train.log", " ".join(command) + " >> train.log 2>&1",
        "echo $? > exit.code",
    ]) + "\n", newline="\n")

    host, remote = shlex.quote(a.host), f"{a.remote_dir}/runs/{a.run}"
    # iqrsh starts the job in the home directory, not the login shell's. It
    # runs inside tmux on the login node, so a dropped SSH connection does not
    # end the interactive job; remote.sh leaves exit.code when training ends.
    session = f"isct-{a.run}"
    job = (f"tmux kill-session -t {session} 2>/dev/null; rm -f {remote}/exit.code; "
           f"tmux new-session -d -s {session} "
           f"'iqrsh -l h_rt={hours:02d}:{minutes:02d}:00 /bin/bash \"$HOME/{remote}/remote.sh\"'")
    out = local / "out"
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir()
    driver = local / "driver.sh"
    driver.write_text(f"""#!/bin/bash
set -uo pipefail
SOCKET="$HOME/.ssh/cm-isct-{a.host}"
O=(-o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=15 -o ServerAliveCountMax=4 -o "ControlPath=$SOCKET")
SSH="ssh ${{O[*]}}"
# Reuse a live shared connection; replace a stale socket. Throttled logins
# back off for one, two, then four minutes.
if ! ssh "${{O[@]}}" -O check {host} 2>/dev/null; then
  rm -f "$SOCKET"
  for pause in 60 120 240 0; do
    ssh "${{O[@]}}" -o ControlMaster=yes -o ControlPersist=3h -fN {host} && break
    [ $pause = 0 ] && exit 255
    sleep $pause
  done
fi
trap 'ssh "${{O[@]}}" -O exit {host} 2>/dev/null' EXIT
rsync -a -e "$SSH" {shlex.quote(posix(stage, a.wsl))}/ {host}:{shlex.quote(a.remote_dir)}/ || exit 255
ssh "${{O[@]}}" {host} {shlex.quote(job)} || exit 255
# Poll once a minute over the shared connection until the job reports.
until ssh "${{O[@]}}" {host} {shlex.quote(f"test -s {remote}/exit.code")}; do sleep 60; done
status=$(ssh "${{O[@]}}" {host} {shlex.quote(f"cat {remote}/exit.code")})
rsync -a -e "$SSH" {host}:{shlex.quote(remote)}/train.log {host}:{shlex.quote(remote)}/out/ {shlex.quote(posix(out, a.wsl))}/
exit "${{status:-1}}"
""", newline="\n")
    runner = ["wsl.exe", "-d", a.wsl, "--", "bash", posix(driver, a.wsl)] if a.wsl else ["bash", str(driver)]
    status = subprocess.run(runner).returncode
    shutil.rmtree(stage, ignore_errors=True)
    if status:
        raise SystemExit(f"Remote run failed ({status}); see {out}")
    print(f"Fetched {out}")


if __name__ == "__main__":
    main()
