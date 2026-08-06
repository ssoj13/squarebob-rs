#!/usr/bin/env python3
"""
bootstrap.py - Unified local build/test/package script for Squarebob.
Cross-platform, Python 3, stdlib only. Ported from Playa's bootstrap.py and
adapted for this repository's xtask setup.
Commands:
    b(uild)       Build squarebob-rs via xtask
    t(est)        Run workspace tests via xtask
    c(heck)       Format check + clippy via xtask
    cl(ean)       Clean build artifacts
    d(ownload)    Re-fetch bundled OCIO ACES configs into data/ocio/
    pkg(package)  Distribution package via cargo-packager
    h(elp)        Print help
Flags:
    -d, --debug       Debug profile for build/test
    -f, --features    Cargo features for build
    -n, --nocapture   Show test output
Examples:
    python bootstrap.py b
    python bootstrap.py b -d
    python bootstrap.py c
"""

from __future__ import annotations
import argparse
import os
import platform
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT_DIR = Path(__file__).parent.resolve()
IS_WINDOWS = platform.system() == "Windows"
CARGO_TOOLS = [
	("cargo-binstall", ["cargo", "binstall", "--version"], ["cargo", "install", "cargo-binstall"]),
	(
		"cargo-packager",
		["cargo", "packager", "--version"],
		["cargo", "binstall", "cargo-packager", "--version", "0.11.7", "--no-confirm"],
	),
]


class C:
	RST = "\033[0m"
	RED = "\033[91m"
	GRN = "\033[92m"
	YLW = "\033[93m"
	CYN = "\033[96m"
	WHT = "\033[97m"

	@classmethod
	def init(cls) -> None:
		if IS_WINDOWS:
			os.system("")


def fmt_time(ms: float) -> str:
	if ms < 1000:
		return f"{ms:.0f}ms"
	if ms < 60000:
		return f"{ms / 1000:.1f}s"
	mins = int(ms // 60000)
	secs = (ms % 60000) / 1000
	return f"{mins}m{secs:.0f}s"


def header(text: str) -> None:
	line = "=" * 60
	print(f"\n{C.CYN}{line}\n{text}\n{line}{C.RST}")


def step(text: str) -> None:
	print(f"  {C.WHT}{text}{C.RST}")


def ok(text: str) -> None:
	print(f"  {C.GRN}[OK] {text}{C.RST}")


def warn(text: str) -> None:
	print(f"  {C.YLW}[WARN] {text}{C.RST}")


def err(text: str) -> None:
	print(f"  {C.RED}[ERR] {text}{C.RST}")


def run(args: list[str], cwd: Path | None = None, capture: bool = False) -> tuple[int, str, float]:
	start = time.perf_counter()
	result = subprocess.run(args, cwd=cwd or ROOT_DIR, capture_output=capture, text=True)
	elapsed_ms = (time.perf_counter() - start) * 1000
	output = (result.stdout or "") + (result.stderr or "") if capture else ""
	return result.returncode, output, elapsed_ms


def which(cmd: str) -> Path | None:
	found = shutil.which(cmd)
	return Path(found) if found else None


def cmd_exists(args: list[str]) -> bool:
	try:
		return subprocess.run(args, capture_output=True, timeout=10).returncode == 0
	except (subprocess.SubprocessError, FileNotFoundError):
		return False


def check_cargo() -> bool:
	if not which("cargo"):
		err("Rust/Cargo not found")
		step("Install Rust from https://rustup.rs/")
		return False
	return True


def setup_vs_env() -> None:
	if not IS_WINDOWS:
		return
	step("Setting up Visual Studio environment...")
	vswhere = Path(os.environ.get("ProgramFiles(x86)", "")) / "Microsoft Visual Studio" / "Installer" / "vswhere.exe"
	if not vswhere.exists():
		warn("vswhere.exe not found; xtask can still use vcv-rs for build/check/clippy")
		return
	result = subprocess.run(
		[str(vswhere), "-latest", "-property", "installationPath"],
		capture_output=True,
		text=True,
	)
	install_path = result.stdout.strip()
	if not install_path:
		warn("Visual Studio installation not found")
		return
	vcvars = Path(install_path) / "VC" / "Auxiliary" / "Build" / "vcvars64.bat"
	if not vcvars.exists():
		warn("vcvars64.bat not found")
		return
	code, output, _ = run(["cmd", "/c", f'"{vcvars}" && set'], capture=True)
	if code != 0:
		warn("Visual Studio environment not configured")
		return
	for line in output.splitlines():
		match = re.match(r"^([^=]+)=(.*)$", line)
		if match:
			os.environ[match.group(1)] = match.group(2)
	ok("Visual Studio environment")


def fix_libclang() -> None:
	libclang = os.environ.get("LIBCLANG_PATH", "")
	if libclang and re.search(r"esp|xtensa", libclang, re.IGNORECASE):
		warn("Clearing LIBCLANG_PATH (ESP/Xtensa clang breaks bindgen/MSVC)")
		del os.environ["LIBCLANG_PATH"]


def setup_env(include_vs: bool = False) -> None:
	if include_vs:
		setup_vs_env()
	fix_libclang()
	print()


def ensure_cargo_tools() -> bool:
	step("Checking cargo tools...")
	for i, (name, check_cmd, install_cmd) in enumerate(CARGO_TOOLS, 1):
		if cmd_exists(check_cmd):
			ok(f"[{i}/{len(CARGO_TOOLS)}] {name}")
			continue
		step(f"[{i}/{len(CARGO_TOOLS)}] Installing {name}...")
		code, _, _ = run(install_cmd)
		if code != 0 and name != "cargo-binstall":
			code, _, _ = run(["cargo", "install", name])
		if code != 0:
			err(f"Failed to install {name}")
			return False
		ok(f"{name} installed")
	print()
	return True


def xtask_cmd(*args: str) -> list[str]:
	return ["cargo", "run", "-p", "xtask", "--", *args]


def run_build(args: argparse.Namespace) -> int:
	header("BUILD")
	cmd = xtask_cmd("build")
	if args.debug:
		cmd.append("--debug")
		step("Mode: debug")
	else:
		step("Mode: release")
	if args.features:
		cmd.extend(["--features", args.features])
		step(f"Features: {args.features}")
	print()
	code, _, elapsed = run(cmd)
	if code == 0:
		ok(f"Build successful ({fmt_time(elapsed)})")
	else:
		err("Build failed")
	print()
	return code


def run_test(args: argparse.Namespace) -> int:
	header("TEST")
	cmd = xtask_cmd("test")
	if args.debug:
		cmd.append("--debug")
	if args.nocapture:
		cmd.append("--nocapture")
	code, _, elapsed = run(cmd)
	if code == 0:
		ok(f"Tests passed ({fmt_time(elapsed)})")
	else:
		err("Tests failed")
	print()
	return code


def run_check(_args: argparse.Namespace) -> int:
	header("CHECK")
	passed = True
	step("Checking formatting...")
	code, _, elapsed = run(["cargo", "fmt", "--check", "-p", "xtask", "-p", "media-encoder", "-p", "squarebob-rs"])
	if code == 0:
		ok(f"Format OK ({fmt_time(elapsed)})")
	else:
		err("Format check failed")
		passed = False
	print()
	step("Running clippy via xtask environment...")
	code, _, elapsed = run(xtask_cmd("clippy", "--workspace", "--all-targets", "--", "-D", "warnings"))
	if code == 0:
		ok(f"Clippy OK ({fmt_time(elapsed)})")
	else:
		err("Clippy failed")
		passed = False
	print()
	if passed:
		ok("All checks passed")
	else:
		err("Some checks failed")
	print()
	return 0 if passed else 1


def run_clean(_args: argparse.Namespace) -> int:
	header("CLEAN")
	code, _, elapsed = run(["cargo", "clean"])
	if code == 0:
		ok(f"Clean complete ({fmt_time(elapsed)})")
	else:
		err("Clean failed")
	print()
	return code


OCIO_ACES_REPO = "AcademySoftwareFoundation/OpenColorIO-Config-ACES"
# Pinned OCIO ACES configs shipped in `data/ocio/`, keyed by the
# GitHub release tag they live on. Each entry is `(tag, [asset, …])`.
# Keep this list in lockstep with the files committed under
# `data/ocio/` so the downloader stays idempotent.
#
# * `v2.1.0-v2.2.0` → ACES 1.3 / OCIO v2.4 configs (Studio + CG).
#   Compact (~100 KB combined) and supported by every OCIO 2.x runtime.
# * `v4.0.0`        → ACES 2.0 / OCIO v2.5 Studio-all-views config.
#   The richest looks + view catalogue available; needs OCIO 2.5+.
OCIO_ACES_BUNDLED: list[tuple[str, list[str]]] = [
	(
		"v4.0.0",
		["studio-config-all-views-v4.0.0_aces-v2.0_ocio-v2.5.ocio"],
	),
]


def run_download(_args: argparse.Namespace) -> int:
	"""Re-fetch the bundled OCIO ACES configs into ``data/ocio/``.
	Idempotent — skips assets that already exist with a non-zero
	size. Pass ``--force`` to clobber existing files.
	Two backends, tried in order:
	1. ``gh release download`` — preferred, picks up the user's
	   authenticated GitHub access and handles retries cleanly.
	2. ``urllib.request`` — pure-stdlib fallback when ``gh`` is
	   missing. No checksums (the OpenColorIO-Config-ACES release
	   page doesn't publish one for individual ``.ocio`` files),
	   but a final non-empty + minimum-size sanity check catches
	   most truncated downloads.
	"""
	header("DOWNLOAD OCIO ACES CONFIGS")
	target_dir = ROOT_DIR / "data" / "ocio"
	target_dir.mkdir(parents=True, exist_ok=True)
	print(f"  {C.WHT}Target: {target_dir}{C.RST}")
	pending: list[tuple[str, str]] = []
	for tag, assets in OCIO_ACES_BUNDLED:
		for name in assets:
			out = target_dir / name
			if out.exists() and out.stat().st_size > 0:
				size_kb = out.stat().st_size / 1024.0
				print(f"  [skip] {name} ({size_kb:.1f} KB already present)")
				continue
			pending.append((tag, name))
	if not pending:
		print(f"  {C.GRN}[OK] all bundled OCIO configs already present{C.RST}")
		return 0
	# Group pending downloads by release tag so we can issue one
	# `gh release download` per tag instead of N.
	by_tag: dict[str, list[str]] = {}
	for tag, name in pending:
		by_tag.setdefault(tag, []).append(name)
	used_urllib = False
	if cmd_exists(["gh", "--version"]):
		gh_ok = True
		for tag, names in by_tag.items():
			cmd = ["gh", "release", "download", tag, "-R", OCIO_ACES_REPO, "-D", str(target_dir)]
			for name in names:
				cmd.extend(["-p", name])
			print(f"  via gh: {tag} → {len(names)} asset(s)...")
			rc = subprocess.run(cmd, cwd=ROOT_DIR).returncode
			if rc != 0:
				print(f"  {C.YLW}[warn] gh download {tag} failed (rc={rc}){C.RST}")
				gh_ok = False
				break
		if gh_ok:
			print(f"  {C.GRN}[OK] downloaded via gh{C.RST}")
			return 0
		used_urllib = True
	if not cmd_exists(["gh", "--version"]) or used_urllib:
		import urllib.request

		for tag, name in pending:
			url = f"https://github.com/{OCIO_ACES_REPO}/releases/download/{tag}/{name}"
			out = target_dir / name
			print(f"  via urllib: {tag}/{name}")
			try:
				with urllib.request.urlopen(url, timeout=60) as r, open(out, "wb") as f:
					shutil.copyfileobj(r, f)
			except Exception as e:
				print(f"  {C.RED}[fail] {name}: {e}{C.RST}")
				return 1
			if out.stat().st_size < 1024:
				print(f"  {C.RED}[fail] {name}: suspiciously small ({out.stat().st_size} B){C.RST}")
				return 1
		print(f"  {C.GRN}[OK] downloaded via urllib{C.RST}")
		return 0
	return 0


def run_package(_args: argparse.Namespace) -> int:
	header("PACKAGE")
	if not ensure_cargo_tools():
		return 1
	code, _, elapsed = run(["cargo", "packager", "--release"])
	if code == 0:
		ok(f"Package complete ({fmt_time(elapsed)})")
	else:
		err("Packaging failed")
	print()
	return code


def run_xtask(extra_args: list[str]) -> int:
	code, _, _ = run(xtask_cmd(*extra_args))
	return code


HELP_TEXT = """
SQUAREBOB BUILD SYSTEM
COMMANDS
  b       build via xtask
  t       test via xtask
  c       cargo fmt --check + xtask clippy
  cl      cargo clean
  d       re-fetch bundled OCIO ACES configs into data/ocio/
  pkg     package via cargo-packager
  h       help
OPTIONS
  -d, --debug       debug profile for build/test
  -f, --features    cargo features for build
  -n, --nocapture   show test output
XTASK PASSTHROUGH
  changelog, tag-dev, tag-rel, pr, deploy, wipe, wipe-wf, check, clippy, build, test
EXAMPLES
  python bootstrap.py b
  python bootstrap.py b -d
  python bootstrap.py c
  python bootstrap.py clippy --workspace --all-targets -- -D warnings
"""
COMMANDS = ["b", "t", "c", "cl", "d", "pkg", "h"]
XTASK_COMMANDS = {
	"build",
	"check",
	"clippy",
	"test",
	"changelog",
	"tag-dev",
	"tag-rel",
	"pr",
	"deploy",
	"wipe",
	"wipe-wf",
}


def main() -> int:
	if hasattr(sys.stdout, "reconfigure"):
		sys.stdout.reconfigure(line_buffering=True)
	C.init()
	if len(sys.argv) > 1 and sys.argv[1] in XTASK_COMMANDS:
		if not check_cargo():
			return 1
		setup_env()
		return run_xtask(sys.argv[1:])
	parser = argparse.ArgumentParser(
		description="Squarebob build system",
		formatter_class=argparse.RawDescriptionHelpFormatter,
	)
	parser.add_argument(
		"command",
		nargs="?",
		choices=COMMANDS,
		default="h",
		help="b, t, c, cl, d, pkg, h",
	)
	parser.add_argument("-d", "--debug", action="store_true", help="Debug mode")
	parser.add_argument("-f", "--features", help="Cargo features")
	parser.add_argument("-n", "--nocapture", action="store_true", help="Show test output")
	args = parser.parse_args()
	if args.command == "h":
		print(HELP_TEXT)
		return 0
	if not check_cargo():
		return 1
	setup_env(include_vs=args.command == "pkg")
	dispatch = {
		"b": run_build,
		"t": run_test,
		"c": run_check,
		"cl": run_clean,
		"d": run_download,
		"pkg": run_package,
	}
	handler = dispatch.get(args.command)
	if handler:
		return handler(args)
	print(HELP_TEXT)
	return 0


if __name__ == "__main__":
	sys.exit(main())
