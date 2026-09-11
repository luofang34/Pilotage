"""Check complete coverage at geographic tile boundaries."""
import sqlite3
import sys
import tempfile
import unittest
from pathlib import Path

sys.dont_write_bytecode = True
from coastline_tile_bounds import complete_tile_plan, prune_archive, tile_rectangle


class TileBoundaryTests(unittest.TestCase):
    def band(self):
        return {"min_zoom": 6, "max_zoom": 10, "min_lon_deg": -81,
                "max_lon_deg": -72, "min_lat_deg": 37.5, "max_lat_deg": 43.5}

    def test_regional_polygons_cover_complete_tiles(self):
        original = self.band()
        band = complete_tile_plan({"bands": [original]})["bands"][0]
        self.assertEqual(band["tile_rectangle"], [17, 23, 20, 25])
        self.assertLess(band["min_lon_deg"], original["min_lon_deg"])
        self.assertGreater(band["max_lat_deg"], original["max_lat_deg"])
        self.assertEqual(tile_rectangle(band), band["tile_rectangle"])

    def test_world_boundary_does_not_create_extra_tiles(self):
        band = {**self.band(), "min_zoom": 0, "min_lon_deg": -180,
                "max_lon_deg": 180, "min_lat_deg": -85.0511287798066,
                "max_lat_deg": 85.0511287798066}
        self.assertEqual(tile_rectangle(band), [0, 0, 1, 1])

    def test_tile_boundary_remains_exact_at_each_detail_level(self):
        for zoom in range(16):
            band = {**self.band(), "min_zoom": zoom}
            expanded = complete_tile_plan({"bands": [band]})["bands"][0]
            self.assertEqual(tile_rectangle(expanded), expanded["tile_rectangle"])

    def test_buffer_fragments_are_removed_with_tms_rows(self):
        plan = complete_tile_plan({"bands": [self.band()]})
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "tiles.mbtiles"
            with sqlite3.connect(archive) as connection:
                connection.execute("CREATE TABLE tiles (zoom_level, tile_column, tile_row)")
                connection.executemany("INSERT INTO tiles VALUES (?, ?, ?)",
                                       [(6, 17, 39), (6, 19, 40), (6, 16, 40),
                                        (6, 20, 40), (6, 17, 38), (6, 17, 41),
                                        (7, 34, 78), (7, 33, 78), (5, 0, 0)])
            prune_archive(archive, plan)
            with sqlite3.connect(archive) as connection:
                self.assertEqual(connection.execute("SELECT * FROM tiles ORDER BY 1, 2").fetchall(),
                                 [(5, 0, 0), (6, 17, 39), (6, 19, 40), (7, 34, 78)])


if __name__ == "__main__":
    unittest.main()
