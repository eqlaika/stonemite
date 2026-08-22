import unittest

from version import CalVer


class CalVerTests(unittest.TestCase):
    def test_public_and_internal_versions(self) -> None:
        cases = {
            "2026.08.22": "2026.8.2200",
            "2026.08.22.1": "2026.8.2201",
            "2026.08.22.99": "2026.8.2299",
            "2026.08.23": "2026.8.2300",
        }
        for public, internal in cases.items():
            with self.subTest(public=public):
                parsed = CalVer.parse(public)
                self.assertEqual(parsed.public, public)
                self.assertEqual(parsed.cargo, internal)

    def test_ordering(self) -> None:
        versions = [
            CalVer.parse(value)
            for value in (
                "2026.08.22",
                "2026.08.22.1",
                "2026.08.22.2",
                "2026.08.23",
                "2026.09.01",
                "2027.01.01",
            )
        ]
        self.assertEqual(versions, sorted(versions))
        self.assertEqual(len(versions), len(set(versions)))

    def test_rejects_noncanonical_and_impossible_versions(self) -> None:
        for value in (
            "2026.8.22",
            "2026.08.22.0",
            "2026.08.22.01",
            "2026.08.22.100",
            "2026.02.29",
            "2026.13.01",
            "0.5.0",
            "2026.08.22-alpha",
        ):
            with self.subTest(value=value):
                with self.assertRaises(ValueError):
                    CalVer.parse(value)
        self.assertEqual(CalVer.parse("2028.02.29").public, "2028.02.29")
        self.assertEqual(CalVer.parse("2000.02.29").public, "2000.02.29")
        with self.assertRaises(ValueError):
            CalVer.parse("2100.02.29")


if __name__ == "__main__":
    unittest.main()
