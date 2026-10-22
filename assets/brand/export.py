"""Export the editable brand SVGs as PNGs. Requires CairoSVG and the brand fonts."""

from pathlib import Path
import sys


ASSET_WIDTHS = {
    "logo": 1024,
    "logo-dark": 1024,
    "logo-light": 1024,
    "avatar": 512,
    "banner.en": 1600,
    "banner.zh-CN": 1600,
}


def main() -> int:
    try:
        import cairosvg
    except ImportError:
        print("Install CairoSVG to export PNGs; see assets/brand/README.md.", file=sys.stderr)
        return 1

    brand_directory = Path(__file__).resolve().parent
    for name, width in ASSET_WIDTHS.items():
        source = brand_directory / f"{name}.svg"
        destination = brand_directory / f"{name}.png"
        cairosvg.svg2png(
            url=str(source),
            write_to=str(destination),
            output_width=width,
        )
        print(f"{source.name} -> {destination.name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
