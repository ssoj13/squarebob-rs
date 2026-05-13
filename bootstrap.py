#!/usr/bin/env python3
"""
bootstrap.py - Unified local build/test/package script for Squarebob.

Cross-platform, Python 3, stdlib only. Ported from Playa's bootstrap.py and
adapted for this repository's xtask/vcpkg setup.

Commands:
    b(uild)       Build squarebob-rs via xtask
    t(est)        Run workspace tests via xtask
    c(heck)       Format check + clippy via xtask
    cl(ean)       Clean build artifacts
    deps          Install pinned vcpkg manifest dependencies
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
    python bootstrap.py deps
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
IS_MACOS = platform.system() == "Darwin"
IS_LINUX = platform.system() == "Linux"

DEFAULT_VCPKG_ROOT = Path("C:/vcpkg") if IS_WINDOWS else Path.home() / "vcpkg"
LOCAL_VCPKG_ROOT = ROOT_DIR / ".vcpkg"


def default_triplet() -> str:
    if IS_WINDOWS:
        return "x64-windows-static-md-release"
    if IS_MACOS:
        machine = platform.machine().lower()
        return "arm64-osx-release" if machine in ("arm64", "aarch64") else "x64-osx-release"
    if IS_LINUX:
        return "x64-linux-release"
    return ""


DEFAULT_TRIPLET = default_triplet()

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


def env_set(name: str, value: str | os.PathLike[str]) -> None:
    os.environ[name] = os.fspath(value)


def setup_vcpkg() -> None:
    triplet = os.environ.get("VCPKGRS_TRIPLET") or DEFAULT_TRIPLET
    manifest_lib = LOCAL_VCPKG_ROOT / "installed" / triplet / "lib" if triplet else None

    if manifest_lib and manifest_lib.exists():
        env_set("VCPKG_ROOT", LOCAL_VCPKG_ROOT)
        ok(f"manifest vcpkg: {LOCAL_VCPKG_ROOT}")
    elif not os.environ.get("VCPKG_ROOT"):
        if DEFAULT_VCPKG_ROOT.exists():
            env_set("VCPKG_ROOT", DEFAULT_VCPKG_ROOT)
            ok(f"vcpkg: {DEFAULT_VCPKG_ROOT}")
        else:
            warn(f"vcpkg not found at {DEFAULT_VCPKG_ROOT}")

    if triplet and not os.environ.get("VCPKGRS_TRIPLET"):
        env_set("VCPKGRS_TRIPLET", triplet)

    vcpkg_root = os.environ.get("VCPKG_ROOT")
    if vcpkg_root and triplet:
        pkg_config = Path(vcpkg_root) / "installed" / triplet / "lib" / "pkgconfig"
        old = os.environ.get("PKG_CONFIG_PATH", "")
        paths = [str(pkg_config)]
        if old:
            paths.append(old)
        env_set("PKG_CONFIG_PATH", os.pathsep.join(paths))
        ok(f"triplet: {triplet}")


def setup_vs_env() -> None:
    if not IS_WINDOWS:
        return

    step("Setting up Visual Studio environment...")
    vswhere = (
        Path(os.environ.get("ProgramFiles(x86)", ""))
        / "Microsoft Visual Studio"
        / "Installer"
        / "vswhere.exe"
    )
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
    setup_vcpkg()
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


def run_deps(_args: argparse.Namespace) -> int:
    header("VCPKG DEPS")
    if not which("vcpkg"):
        vcpkg_exe = DEFAULT_VCPKG_ROOT / ("vcpkg.exe" if IS_WINDOWS else "vcpkg")
        if vcpkg_exe.exists():
            vcpkg = str(vcpkg_exe)
        else:
            err("vcpkg executable not found")
            step(f"Expected at {vcpkg_exe} or in PATH")
            return 1
    else:
        vcpkg = "vcpkg"

    triplet = os.environ.get("VCPKGRS_TRIPLET") or DEFAULT_TRIPLET
    cmd = [
        vcpkg,
        "install",
        "--x-manifest-root",
        ".",
        "--x-install-root",
        ".vcpkg/installed",
        "--triplet",
        triplet,
    ]
    step("Installing pinned vcpkg manifest dependencies...")
    code, _, elapsed = run(cmd)
    if code == 0:
        ok(f"vcpkg dependencies installed ({fmt_time(elapsed)})")
    else:
        err("vcpkg install failed")
    print()
    return code


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
  deps    install pinned vcpkg manifest dependencies
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
  python bootstrap.py deps
  python bootstrap.py clippy --workspace --all-targets -- -D warnings
"""

COMMANDS = ["b", "t", "c", "cl", "deps", "pkg", "h"]
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
        help="b, t, c, cl, deps, pkg, h",
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
        "deps": run_deps,
        "pkg": run_package,
    }
    handler = dispatch.get(args.command)
    if handler:
        return handler(args)

    print(HELP_TEXT)
    return 0


if __name__ == "__main__":
    sys.exit(main())
