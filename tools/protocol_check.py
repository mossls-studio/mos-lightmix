"""mos_灯光混合 helper 协议自测：不需要打开 EffectCraft。

模拟宿主，把 helper 当子进程驱动，检查这次修复的几条关键行为：
  1. hello / describe 正常，manifest 里 padding=true
  2. padding 按缩放缩放（size*0.75*scale，向上取整）
  3. 不需要背景层时，render 直接回 done
  4. 需要背景层、宿主故意不回 image 时，helper 在 MOS_LIGHTWRAP_LAYER_TIMEOUT 秒后回 error，
     且进程还活着（老版本会永远卡在读 stdin 上）
  5. 宿主不回 image 期间到达的 padding 请求也会被应答（老版本会被静默吞掉）
  6. 同一时间/同一图层的连续 render 命中缓存，第二次不再发 layer_input（拖滑杆提速）

用法： python tools/protocol_check.py [helper.exe 的路径]
"""

import json
import os
import queue
import subprocess
import sys
import tempfile
import threading
import time

PROTO = 1
EFFECT = "mos.lightwrap"
W = 64
H = 64


class Helper:
    def __init__(self, exe, frames, env_extra):
        env = dict(os.environ)
        env.update(env_extra)
        self.proc = subprocess.Popen(
            [exe, "--effectcraft-helper"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            bufsize=0,
            env=env,
        )
        self.q = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for raw in self.proc.stdout:
            line = raw.decode("utf-8", "replace").strip()
            if not line:
                continue
            if line.startswith("@ec-helper "):
                line = line[len("@ec-helper "):]
            try:
                self.q.put(json.loads(line))
            except ValueError:
                self.q.put({"op": "@@raw", "line": line})

    def send(self, obj):
        self.proc.stdin.write((json.dumps(obj) + "\n").encode("utf-8"))
        self.proc.stdin.flush()

    def recv(self, timeout=10.0):
        return self.q.get(timeout=timeout)

    def drain(self):
        while True:
            try:
                self.q.get_nowait()
            except queue.Empty:
                return

    def close(self):
        try:
            self.proc.stdin.close()
        except OSError:
            pass
        try:
            self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.proc.kill()


def flat(background=-1, size=20.0, blur=0.0, render=1, mask_style=0, mask_layer=-1):
    f = [0.0] * 21
    f[0] = float(render)        # 渲染
    f[1] = float(background)    # 背景层
    f[6] = float(blur)          # 背景模糊
    f[7] = 1.0                  # 缩放背景
    f[8] = float(size)          # 尺寸
    f[9] = 0.0                  # 混合模式
    f[10] = 100.0               # 亮度
    f[12] = float(mask_style)   # 遮罩样式
    f[17] = float(mask_layer)   # 遮罩图层
    return f


def render_req(instance, t, background=-1, size=8.0):
    return {
        "op": "render",
        "effect": EFFECT,
        "instance": instance,
        "time": t,
        "frame": {"offset": 0, "width": W, "height": H},
        "layer_size": [float(W), float(H)],
        "flat": flat(background=background, size=size),
    }


def image_req(width, height, origin=(0.0, 0.0), scale=1.0):
    return {
        "op": "image",
        "image": {
            "offset": 0,
            "width": width,
            "height": height,
            "origin": list(origin),
            "scale": scale,
        },
    }


results = []


def check(name, ok, detail=""):
    results.append((name, ok, detail))
    print(("  PASS  " if ok else "  FAIL  ") + name + (("  <- " + detail) if detail else ""))


def main():
    here = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    exe = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        os.environ.get("APPDATA", ""), "EffectCraft", "helpers",
        "effectcraft-helper-mos-lightmix.exe",
    )
    if not os.path.isfile(exe):
        print("找不到 helper：" + exe)
        return 2
    print("helper: " + exe)
    print("layer timeout = 2 s（测试用）, cache = 3 s\n")

    tmpdir = tempfile.mkdtemp(prefix="lwf-check-")
    frames = os.path.join(tmpdir, "frames.bin")
    with open(frames, "wb") as fh:                      # 全 1.0 的 RGBA（预乘）
        fh.write(b"\x00\x00\x80\x3f" * (W * H * 4))

    h = Helper(exe, frames, {
        "MOS_LIGHTWRAP_LAYER_TIMEOUT": "2",
        "MOS_LIGHTWRAP_LAYER_CACHE_SECS": "3",
    })
    try:
        # 1. hello / describe
        h.send({"op": "hello", "version": PROTO, "frames": frames})
        m = h.recv()
        check("hello 应答", m.get("op") == "hello", str(m))
        h.send({"op": "describe"})
        m = h.recv()
        effects = m.get("effects") or []
        ok = m.get("op") == "effects" and effects and effects[0].get("manifest", {}).get("id") == EFFECT
        check("describe 返回 mos.lightwrap 且 padding=true",
              ok and effects[0].get("padding") is True, str(m)[:160])

        # 2. padding 按缩放
        h.send({"op": "padding", "effect": EFFECT, "instance": 1, "time": 0.0, "scale": 1.0,
                "layer_size": [float(W), float(H)], "flat": flat(size=20.0)})
        m1 = h.recv()
        check("padding scale=1 → 15", m1.get("pixels") == 15, str(m1))
        h.send({"op": "padding", "effect": EFFECT, "instance": 1, "time": 0.0, "scale": 0.5,
                "layer_size": [float(W), float(H)], "flat": flat(size=20.0)})
        m2 = h.recv()
        check("padding scale=0.5 → 8（老版本恒为 15）", m2.get("pixels") == 8, str(m2))

        # 3. 不需要背景层：直接 done
        h.drain()
        t0 = time.time()
        h.send(render_req(1, 0.0, background=-1))
        m = h.recv(timeout=10)
        check("无背景层 render → done",
              m.get("op") == "done" and time.time() - t0 < 5, "%s  %.2fs" % (m, time.time() - t0))

        # 4. 需要背景层但宿主不回 image：应超时报 error，进程不死
        h.drain()
        t0 = time.time()
        h.send(render_req(1, 1.0, background=0))
        first = h.recv(timeout=5)
        check("有背景层 render 先发 layer_input", first.get("op") == "layer_input", str(first))
        m = h.recv(timeout=15)          # 故意不回 image
        waited = time.time() - t0
        check("等不到 image 时 %.1fs 内回 error（不会永远挂着）" % waited,
              m.get("op") == "error" and waited < 6, str(m))

        h.send({"op": "ping"})
        m = h.recv(timeout=5)
        check("超时后进程仍存活（pong）", m.get("op") == "pong", str(m))

        # 5. 等待 image 期间到达的 padding 要被应答
        h.drain()
        h.send(render_req(1, 2.0, background=0))
        first = h.recv(timeout=5)
        h.send({"op": "padding", "effect": EFFECT, "instance": 1, "time": 2.0, "scale": 1.0,
                "layer_size": [float(W), float(H)], "flat": flat(size=20.0)})
        got = None
        for _ in range(2):
            m = h.recv(timeout=5)
            if m.get("op") == "padding":
                got = m
                break
        check("等待 image 时 padding 有应答（老版本会被吞掉）", got is not None and got.get("pixels") == 15,
              str(got))

        # 6. 缓存：同一时间点连续两次带背景层 render，第二次不再要 layer_input
        h.drain()
        h.send(render_req(1, 3.0, background=0))
        m = h.recv(timeout=5)
        if m.get("op") == "layer_input":
            h.send(image_req(W, H))
            m = h.recv(timeout=10)
        check("第一次带背景层 render 完成", m.get("op") == "done", str(m))
        h.send(render_req(1, 3.0, background=0))
        m = h.recv(timeout=10)
        check("第二次同时间点 render 直接 done（命中缓存）", m.get("op") == "done", str(m))

        h.send({"op": "shutdown"})
    finally:
        h.close()

    bad = [r for r in results if not r[1]]
    print("\n%d/%d 通过" % (len(results) - len(bad), len(results)))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
