"""Audit viewer token references and CSS boundaries against installed Primer."""
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FRONTEND = ROOT / 'packages/frontend'
source_files = sorted(p for p in (FRONTEND / 'src').rglob('*') if p.suffix in {'.css', '.ts', '.tsx'})
source = '\n'.join(p.read_text() for p in source_files)
primer_files = sorted((FRONTEND / 'node_modules/@primer/primitives/dist/css').rglob('*.css'))
primer_files += [FRONTEND / 'node_modules/@primer/css/dist/primer.css']
primer = '\n'.join(p.read_text() for p in primer_files)
definitions = set(re.findall(r'(--[\w-]+)\s*:', primer))
references = set(re.findall(r'var\(\s*(--[\w-]+)\s*[,)]', source))
references.update(re.findall(r"['\"](--[\w-]+)['\"]", source))
# Both Canvas and GPU derive these upstream names from the same map palette.
for hue in ['green', 'gray', 'blue', 'brown', 'yellow', 'orange', 'auburn', 'purple', 'plum', 'olive', 'pine', 'pink']:
    for level in ['emphasis', 'muted']:
        references.add(f'--data-{hue}-color-{level}')
custom_definitions = sorted(set(re.findall(r'(--[\w-]+)\s*:', source)))
missing = sorted(references - definitions)
css = (FRONTEND / 'src/viewer.css').read_text()
selectors = re.findall(r'([^{}]+)\{', re.sub(r'/\*.*?\*/', '', css, flags=re.S))
allowed = {'.app-shell', '.app-main', '.PageLayout-content', '.map-stage', '.map-stage > canvas'}
unexpected = [selector.strip() for selector in selectors if selector.strip() not in allowed]
result = {'primerCssVersion': json.loads((FRONTEND / 'node_modules/@primer/css/package.json').read_text())['version'], 'primerPrimitivesVersion': json.loads((FRONTEND / 'node_modules/@primer/primitives/package.json').read_text())['version'], 'sourceFiles': len(source_files), 'referencedTokens': sorted(references), 'customTokenDefinitions': custom_definitions, 'undefinedTokens': missing, 'cssSelectorsOutsideViewportAndMap': unexpected, 'scope': 'Installed Primer CSS/token consistency; not pixel equality with github.com. Native range input and domain map are outside Primer component equivalence.'}
print(json.dumps(result, indent=2))
if custom_definitions or missing or unexpected:
    raise SystemExit(1)
