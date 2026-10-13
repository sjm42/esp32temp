import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "check_ota_size.py"
SPEC = importlib.util.spec_from_file_location("check_ota_size", SCRIPT)
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


class OtaImageSizeTests(unittest.TestCase):
    def test_uses_smallest_slot_and_ignores_factory_partition(self):
        with tempfile.TemporaryDirectory() as directory:
            table = Path(directory) / "partitions.csv"
            table.write_text("# Test table\nfactory,app,factory,,4K\nota_0,app,ota_0,,2M\nota_1,0,0x11,,0x180000\n")
            self.assertEqual(CHECK.ota_capacity(table), 0x180000)
            table.write_text("nvs,data,nvs,,16K\n")
            with self.assertRaises(ValueError):
                CHECK.ota_capacity(table)

    def test_cli_accepts_exact_fit_and_rejects_empty_oversized_or_missing_images(self):
        with tempfile.TemporaryDirectory() as directory:
            table = Path(directory) / "partitions.csv"
            table.write_text("ota_0,app,ota_0,,16\nota_1,app,ota_1,,32\n")
            image = Path(directory) / "firmware.bin"
            for size, expected_code in [(0, 1), (16, 0), (17, 1)]:
                image.write_bytes(bytes(size))
                result = subprocess.run([sys.executable, str(SCRIPT), str(image), str(table)], capture_output=True)
                self.assertEqual(result.returncode, expected_code)
            result = subprocess.run([sys.executable, str(SCRIPT), str(image.with_name("missing.bin")), str(table)], capture_output=True)
            self.assertEqual(result.returncode, 1)


if __name__ == "__main__":
    unittest.main()
