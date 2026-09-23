#!/usr/bin/env python3
"""Check the plugin's translation setup.

Fails when:
  * a QML file uses plain I18n.tr( (it never sees the plugin's translations),
  * an I18n.trFor call does not pass the plugin id as a literal (the DMS
    extraction tooling reads call sites, so a variable hides the string),
  * a translation file is not valid JSON or not in DMS's {term: {term: text}}
    shape,
  * a translation key no longer matches any string in the QML,
  * a translation drops or adds a %N placeholder.

Usage: scripts/check-i18n.py [plugin-dir]
"""

import json
import re
import sys
from pathlib import Path

PLUGIN = Path(sys.argv[1] if len(sys.argv) > 1 else "plugin")
PLUGIN_ID = json.loads((PLUGIN / "plugin.json").read_text())["id"]

LITERAL = r'"((?:[^"\\]|\\.)*)"'
TR_FOR = re.compile(r"I18n\.trFor\(\s*" + LITERAL + r"\s*,\s*" + LITERAL)
PLACEHOLDER = re.compile(r"%\d")


def placeholders(text):
    return sorted(PLACEHOLDER.findall(text))


def main():
    errors = []
    terms = set()

    for qml in sorted(PLUGIN.glob("*.qml")):
        source = qml.read_text()
        for number, line in enumerate(source.splitlines(), 1):
            if re.search(r"I18n\.tr\(", line):
                errors.append(f"{qml}:{number}: use I18n.trFor(\"{PLUGIN_ID}\", ...) instead of I18n.tr(")
            for call in re.finditer(r"I18n\.trFor\(\s*([^,]*),", line):
                if call.group(1).strip() != f'"{PLUGIN_ID}"':
                    errors.append(f"{qml}:{number}: trFor must pass the literal id \"{PLUGIN_ID}\"")
        for match in TR_FOR.finditer(source):
            terms.add(json.loads(f'"{match.group(2)}"'))

    translations = sorted((PLUGIN / "translations").glob("*.json"))
    for path in translations:
        try:
            table = json.loads(path.read_text())
        except json.JSONDecodeError as err:
            errors.append(f"{path}: invalid JSON: {err}")
            continue
        for term, bucket in table.items():
            if not isinstance(bucket, dict) or term not in bucket:
                errors.append(f"{path}: {term!r} must map to {{{term!r}: translation}}")
                continue
            if term not in terms:
                errors.append(f"{path}: {term!r} matches no string in the QML")
            if placeholders(term) != placeholders(bucket[term]):
                errors.append(f"{path}: placeholders differ for {term!r}")

    for error in errors:
        print(error)
    print(f"{len(terms)} strings, {len(translations)} translation files, {len(errors)} errors")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
