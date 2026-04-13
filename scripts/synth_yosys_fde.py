#!/usr/bin/env python3

from __future__ import annotations

import argparse
import shutil
import subprocess
import tempfile
from pathlib import Path


FDE_LUT_WIDTH = 4


def repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def quote_yosys_path(path: Path) -> str:
    return '"' + str(path).replace('\\', '/').replace('"', '\\"') + '"'


def default_support_dir(root: Path) -> Path | None:
    import os

    raw_env = os.environ.get("FDE_YOSYS_SUPPORT_DIR")
    if raw_env:
        env_path = Path(raw_env)
        if env_path.is_dir():
            return env_path.resolve()

    sibling_aspen = root.parent / "aspen" / "src-tauri" / "resource" / "yosys-fde"
    if sibling_aspen.is_dir():
        return sibling_aspen.resolve()

    return None


def required_support_files(support_dir: Path) -> dict[str, Path]:
    files = {
        "fdesimlib": support_dir / "fdesimlib.v",
        "bram_lib": support_dir / "brams.txt",
        "bram_map": support_dir / "brams_map.v",
        "techmap": support_dir / "techmap.v",
        "cells_map": support_dir / "cells_map.v",
    }
    missing = [name for name, path in files.items() if not path.is_file()]
    if missing:
        raise SystemExit(
            f"support dir {support_dir} is missing required file(s): {', '.join(missing)}"
        )
    return files


def build_yosys_script(
    support_files: dict[str, Path],
    source_paths: list[Path],
    top_module: str,
    edif_path: Path,
    json_path: Path | None,
) -> str:
    quoted_sources = " ".join(quote_yosys_path(path) for path in source_paths)
    lines = [
        f"read_verilog -lib {quote_yosys_path(support_files['fdesimlib'])}",
        f"read_verilog -sv {quoted_sources}",
        f"hierarchy -check -top {top_module}",
        "proc",
        "flatten -noscopeinfo",
        "memory -nomap",
        "opt_clean",
        f"memory_libmap -lib {quote_yosys_path(support_files['bram_lib'])}",
        f"techmap -map {quote_yosys_path(support_files['bram_map'])}",
        "opt",
        "memory_map",
        "opt -fast",
        "opt -full",
        f"techmap -map {quote_yosys_path(support_files['techmap'])}",
        "simplemap",
        "dfflegalize \\",
        "  -cell $_DFF_N_ 01 \\",
        "  -cell $_DFF_P_ 01 \\",
        "  -cell $_DFFE_PP_ 01 \\",
        "  -cell $_DFFE_PN_ 01 \\",
        "  -cell $_DFF_PN0_ r \\",
        "  -cell $_DFF_PN1_ r \\",
        "  -cell $_DFF_PP0_ r \\",
        "  -cell $_DFF_PP1_ r \\",
        "  -cell $_DFF_NN0_ r \\",
        "  -cell $_DFF_NN1_ r \\",
        "  -cell $_DFF_NP0_ r \\",
        "  -cell $_DFF_NP1_ r",
        f"techmap -D NO_LUT -map {quote_yosys_path(support_files['cells_map'])}",
        "opt",
        "wreduce",
        "clean",
        "dffinit -ff DFFNHQ Q INIT -ff DFFHQ Q INIT -ff EDFFHQ Q INIT -ff DFFRHQ Q INIT -ff DFFSHQ Q INIT -ff DFFNRHQ Q INIT -ff DFFNSHQ Q INIT",
        f"abc -lut {FDE_LUT_WIDTH}",
        "opt",
        "wreduce",
        "clean",
        "maccmap -unmap",
        "techmap",
        "simplemap",
        "opt",
        "wreduce",
        "clean",
        f"abc -lut {FDE_LUT_WIDTH}",
        "opt",
        "wreduce",
        "clean",
        f"techmap -map {quote_yosys_path(support_files['cells_map'])}",
        "opt",
        "check",
        "stat",
        f"write_edif {quote_yosys_path(edif_path)}",
    ]
    if json_path is not None:
        lines.append(f"write_json {quote_yosys_path(json_path)}")
    return "\n".join(lines) + "\n"


def run_yosys(yosys_bin: str, script_path: Path, log_path: Path, work_dir: Path) -> None:
    result = subprocess.run(
        [yosys_bin, "-s", str(script_path)],
        cwd=work_dir,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    log_path.write_text(result.stdout)
    if result.returncode != 0:
        raise SystemExit(
            f"yosys failed with exit code {result.returncode}; see {log_path}"
        )


def build_parser() -> argparse.ArgumentParser:
    root = repo_root()
    parser = argparse.ArgumentParser(
        description="Synthesize Verilog to FDE-compatible EDIF using Aspen's Yosys synthesis flow."
    )
    parser.add_argument("sources", nargs="+", help="Input Verilog/SystemVerilog source files.")
    parser.add_argument("--top", required=True, help="Top module name.")
    parser.add_argument("--out-edf", required=True, help="Output EDIF path.")
    parser.add_argument("--out-json", help="Optional output JSON netlist path.")
    parser.add_argument("--out-script", help="Optional path to keep the generated Yosys script.")
    parser.add_argument(
        "--log",
        help="Optional Yosys log path. Defaults next to the EDIF output.",
    )
    parser.add_argument(
        "--support-dir",
        default=str(default_support_dir(root) or ""),
        help="Directory containing fdesimlib.v, brams.txt, brams_map.v, techmap.v, and cells_map.v.",
    )
    parser.add_argument(
        "--yosys-bin",
        default=shutil.which("yosys") or "yosys",
        help="Yosys executable to run.",
    )
    parser.add_argument(
        "--work-dir",
        help="Optional working directory for temporary files. Defaults to the EDIF output directory.",
    )
    return parser


def main() -> int:
    root = repo_root()
    args = build_parser().parse_args()

    if not args.support_dir:
        raise SystemExit(
            "could not locate Aspen yosys-fde support files; pass --support-dir or set FDE_YOSYS_SUPPORT_DIR"
        )

    if shutil.which(args.yosys_bin) is None and not Path(args.yosys_bin).is_file():
        raise SystemExit(f"yosys binary not found: {args.yosys_bin}")

    source_paths = [Path(source).resolve() for source in args.sources]
    missing_sources = [str(path) for path in source_paths if not path.is_file()]
    if missing_sources:
        raise SystemExit(f"missing source file(s): {', '.join(missing_sources)}")

    support_dir = Path(args.support_dir).resolve()
    support_files = required_support_files(support_dir)

    edif_path = Path(args.out_edf).resolve()
    edif_path.parent.mkdir(parents=True, exist_ok=True)
    json_path = Path(args.out_json).resolve() if args.out_json else None
    if json_path is not None:
        json_path.parent.mkdir(parents=True, exist_ok=True)

    log_path = Path(args.log).resolve() if args.log else edif_path.with_suffix(".yosys.log")
    log_path.parent.mkdir(parents=True, exist_ok=True)

    work_dir = Path(args.work_dir).resolve() if args.work_dir else edif_path.parent
    work_dir.mkdir(parents=True, exist_ok=True)

    script_body = build_yosys_script(
        support_files,
        source_paths,
        args.top,
        edif_path,
        json_path,
    )

    if args.out_script:
        script_path = Path(args.out_script).resolve()
        script_path.parent.mkdir(parents=True, exist_ok=True)
        script_path.write_text(script_body)
        run_yosys(args.yosys_bin, script_path, log_path, work_dir)
    else:
        with tempfile.NamedTemporaryFile(
            mode="w",
            suffix=".ys",
            prefix="fde-yosys-",
            dir=work_dir,
            delete=False,
        ) as handle:
            handle.write(script_body)
            script_path = Path(handle.name)
        try:
            run_yosys(args.yosys_bin, script_path, log_path, work_dir)
        finally:
            script_path.unlink(missing_ok=True)

    print(f"edif={edif_path}")
    if json_path is not None:
        print(f"json={json_path}")
    print(f"log={log_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
