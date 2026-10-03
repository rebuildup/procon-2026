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
field = ET.fromstring(re.search(r'<svg class="hexfield"[^>]*>(.*?)</svg>', source, re.S).group(0))
groups = list(field)
# Preserve native SVG structure: patterns, inherited styles and exact casing.
centers = [(52,44),(132,44),(212,44),(292,44),(92,102),(252,102)]
ranges = [(i,i+1) for i in range(6)]
structure = {'viewBox': '0 0 420 168', 'defs': node(groups[0]),
             'hex': [], 'series': {}, 'reference': [node(e) for e in groups]}
for i, (x,y) in enumerate(centers):
    structure['hex'].append({'center':[x,y], 'halo': node(ET.Element('g', groups[1].attrib)), 'fill':node(ET.Element('g',groups[2].attrib))})
    structure['hex'][-1]['halo']['children'] = [node(groups[1][i])]
    structure['hex'][-1]['fill']['children'] = [node(groups[2][i])]
    a,b = ranges[i]
    key = ['s0','s1','s2','s3','s5','s4'][i]
    structure['series'][key] = {'center':[x,y], 'layers': []}
    for group in [groups[3],groups[4]]:
        layer = node(ET.Element('g',group.attrib));layer['children']=[node(e) for e in list(group)[a:b]]
        structure['series'][key]['layers'].append(layer)
structure_path = ROOT / 'packages/frontend/src/viewer-structure.json'
if '--check' in sys.argv:
    assert json.loads(structure_path.read_text()) == structure, 'Canonical hex structure differs'
else:
    structure_path.write_text(json.dumps(structure,indent=2)+'\n')
target = ROOT / 'packages/frontend/src/viewer-marks.json'
if '--check' in sys.argv:
    assert json.loads(target.read_text()) == result, 'Canonical glyph definitions differ'
else:
    target.write_text(json.dumps(result, indent=2) + '\n')
