#!/usr/bin/env python3
"""Reject application images that do not fit every OTA slot in a partition table."""

import argparse
import csv
from pathlib import Path


def partition_size(value):
    value = value.strip().upper()
    multiplier = 1
    if value.endswith(("K", "M")):
        multiplier = 1024 if value[-1] == "K" else 1024 * 1024
        value = value[:-1]
    size = int(value, 16 if value.startswith("0X") else 10) * multiplier
    if size <= 0:
        raise ValueError("Partition size must be positive")
    return size


def ota_capacity(table):
    slots = []
    for row in csv.reader(line for line in table.read_text().splitlines()
                          if line.strip() and not line.lstrip().startswith("#")):
        if len(row) < 5:
            raise ValueError("Partition table row has fewer than five columns")
        _, kind, subtype, _, size, *_ = (item.strip() for item in row)
        if kind.lower() not in ("app", "0", "0x0", "0x00"):
            continue
        if subtype.startswith("ota_"):
            is_ota = 0 <= int(subtype[4:]) <= 15
        else:
            try:
                is_ota = 0x10 <= int(subtype, 0) <= 0x1F
            except ValueError:
                is_ota = False
        if is_ota:
            slots.append(partition_size(size))
    if not slots:
        raise ValueError("Partition table contains no OTA slots")
    return min(slots)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image", type=Path)
    parser.add_argument("partitions", type=Path)
    args = parser.parse_args()
    try:
        capacity = ota_capacity(args.partitions)
        size = args.image.stat().st_size
        if size == 0 or size > capacity:
            raise ValueError(f"Image is {size} bytes; OTA capacity is {capacity} bytes")
    except (OSError, ValueError) as error:
        parser.exit(1, f"OTA image check failed: {error}\n")
    print(f"{args.image}: {size} / {capacity} bytes ({capacity - size} bytes free)")


if __name__ == "__main__":
    main()
