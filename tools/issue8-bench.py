#!/usr/bin/env python3
"""用 CLI 的逐句查询耗时复测 Issue #8 冻结语料（先预热一轮）。"""

import argparse
import re
import statistics
import subprocess
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("--dict", default="data/generated/dict.qj")
    args = parser.parse_args()
    inputs = [line.split("\t")[1] for line in args.corpus.read_text().splitlines()]
    pattern = re.compile(r"· total (\d+(?:\.\d+)?)(µs|ms|s)")
    samples = []
    for repetition in range(6):
        output = subprocess.check_output(
            [str(args.binary.resolve()), "--dict", args.dict, "--limit", "0", *inputs],
            text=True,
        )
        times = [
            float(value) * {"µs": 0.001, "ms": 1.0, "s": 1000.0}[unit]
            for value, unit in pattern.findall(output)
        ]
        if len(times) != len(inputs):
            raise RuntimeError(f"需要 {len(inputs)} 条计时，实际得到 {len(times)} 条")
        if repetition:
            samples.extend(times)
    samples.sort()
    print(
        f"{len(samples)} queries; median {statistics.median(samples):.2f} ms; "
        f"P95 {samples[int((len(samples) - 1) * 0.95)]:.2f} ms; "
        f"max {samples[-1]:.2f} ms"
    )


if __name__ == "__main__":
    main()
