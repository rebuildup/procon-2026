"""Extract the 36 canonical domain glyphs without modifying proposal HTML."""
import json
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path
ROOT = Path(__file__).resolve().parent.parent
source = (ROOT / 'docs/design/proposals/design-system.html').read_text()
roles = ['plain', 'mountain', 'water', 'road', 'patrol', 'supply', *[f's{i}' for i in range(6)]]
result = {}
def node(element):
    return {'tag': element.tag, 'attrs': dict(element.attrib), 'children': [node(child) for child in element]}
for index, family in enumerate(['ridge', 'peak', 'terrace']):
    start = source.index(f'Family {chr(65 + index)} —')
    # Family headings contain exactly two six-glyph rows before the next section.
    glyphs = re.findall(r'<svg class="mark-cell__glyph"[^>]*>(.*?)</svg>', source[start:], re.S)[:12]
    assert len(glyphs) == 12
    for role, markup in zip(roles, glyphs):
        key = role if family == 'ridge' else f'{family}-{role}'
        result[key] = [node(e) for e in ET.fromstring(f'<svg>{markup}</svg>')]
target = ROOT / 'packages/frontend/src/viewer-marks.json'
if '--check' in sys.argv:
    assert json.loads(target.read_text()) == result, 'Canonical glyph definitions differ'
else:
    target.write_text(json.dumps(result, indent=2) + '\n')
