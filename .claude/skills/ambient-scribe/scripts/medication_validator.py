"""
Medication validator for ambient-scribe.

Validates a list of medication records (per assets/medication-record-schema.json)
for:
    1. Drug name resolution against a configured vocabulary (RxNorm by default;
       offline fallback dictionary included).
    2. Dose unit plausibility for the route (e.g., 500 mg of inhaled albuterol
       is implausible).
    3. Same-class duplicates flagged for clinician review (without blocking).

This script does NOT:
    - Check drug–drug or drug–allergy interactions. That is the EHR's job, and
      duplicating it creates a coordination hazard. The skill notes this in
      references/medications.md.
    - Auto-correct misheard drug names. Misheard tokens are flagged with
      ``unresolved=True`` and surfaced to the clinician.

RxNorm integration:
    By default this script uses an offline mini-dictionary covering common
    drugs to keep it runnable in air-gapped environments. The deploying
    organization can plug in a live RxNorm client by registering a custom
    resolver via ``register_resolver()``. The interface is intentionally small.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from typing import Callable

# ---------------------------------------------------------------------------
# Resolver protocol
# ---------------------------------------------------------------------------

# A resolver maps a raw drug-name string to (resolved_name, rxcui, drug_class)
# or returns None if the name cannot be resolved.
Resolver = Callable[[str], "ResolverResult | None"]


@dataclass(frozen=True)
class ResolverResult:
    name: str
    rxcui: str | None
    drug_class: str | None


# ---------------------------------------------------------------------------
# Offline mini-dictionary
# ---------------------------------------------------------------------------
# Intentionally minimal — enough for the skill to be useful out of the box on
# common cardiovascular, endocrine, psych, GI, respiratory, and antibiotic
# drugs. Production deployments should plug in a real RxNorm resolver.

_OFFLINE_DICT: dict[str, ResolverResult] = {
    # Cardiovascular
    "lisinopril":      ResolverResult("lisinopril",      "29046",  "ACE inhibitor"),
    "enalapril":       ResolverResult("enalapril",       "3827",   "ACE inhibitor"),
    "ramipril":        ResolverResult("ramipril",        "35296",  "ACE inhibitor"),
    "losartan":        ResolverResult("losartan",        "52175",  "ARB"),
    "valsartan":       ResolverResult("valsartan",       "69749",  "ARB"),
    "metoprolol":      ResolverResult("metoprolol",      "6918",   "beta blocker"),
    "atenolol":        ResolverResult("atenolol",        "1202",   "beta blocker"),
    "carvedilol":      ResolverResult("carvedilol",      "20352",  "beta blocker"),
    "amlodipine":      ResolverResult("amlodipine",      "17767",  "calcium channel blocker"),
    "diltiazem":       ResolverResult("diltiazem",       "3443",   "calcium channel blocker"),
    "hydrochlorothiazide": ResolverResult("hydrochlorothiazide", "5487", "thiazide diuretic"),
    "furosemide":      ResolverResult("furosemide",      "4603",   "loop diuretic"),
    "atorvastatin":    ResolverResult("atorvastatin",    "83367",  "statin"),
    "rosuvastatin":    ResolverResult("rosuvastatin",    "301542", "statin"),
    "simvastatin":     ResolverResult("simvastatin",     "36567",  "statin"),
    "warfarin":        ResolverResult("warfarin",        "11289",  "anticoagulant"),
    "apixaban":        ResolverResult("apixaban",        "1364430","anticoagulant"),
    "rivaroxaban":     ResolverResult("rivaroxaban",     "1114195","anticoagulant"),
    "aspirin":         ResolverResult("aspirin",         "1191",   "antiplatelet"),
    "clopidogrel":     ResolverResult("clopidogrel",     "32968",  "antiplatelet"),
    # Endocrine
    "metformin":       ResolverResult("metformin",       "6809",   "biguanide"),
    "glipizide":       ResolverResult("glipizide",       "4821",   "sulfonylurea"),
    "insulin glargine":ResolverResult("insulin glargine","274783", "long-acting insulin"),
    "insulin lispro":  ResolverResult("insulin lispro",  "139825", "rapid-acting insulin"),
    "levothyroxine":   ResolverResult("levothyroxine",   "10582",  "thyroid hormone"),
    "semaglutide":     ResolverResult("semaglutide",     "1991302","GLP-1 agonist"),
    # Psych
    "sertraline":      ResolverResult("sertraline",      "36437",  "SSRI"),
    "fluoxetine":      ResolverResult("fluoxetine",      "4493",   "SSRI"),
    "escitalopram":    ResolverResult("escitalopram",    "321988", "SSRI"),
    "bupropion":       ResolverResult("bupropion",       "42347",  "atypical antidepressant"),
    "trazodone":       ResolverResult("trazodone",       "10737",  "atypical antidepressant"),
    "lorazepam":       ResolverResult("lorazepam",       "6470",   "benzodiazepine"),
    "alprazolam":      ResolverResult("alprazolam",      "596",    "benzodiazepine"),
    # GI
    "omeprazole":      ResolverResult("omeprazole",      "7646",   "PPI"),
    "pantoprazole":    ResolverResult("pantoprazole",    "40790",  "PPI"),
    "ondansetron":     ResolverResult("ondansetron",     "26225",  "antiemetic"),
    # Respiratory
    "albuterol":       ResolverResult("albuterol",       "435",    "SABA"),
    "fluticasone":     ResolverResult("fluticasone",     "41126",  "inhaled corticosteroid"),
    "montelukast":     ResolverResult("montelukast",     "88249",  "leukotriene modifier"),
    # Pain
    "acetaminophen":   ResolverResult("acetaminophen",   "161",    "analgesic"),
    "ibuprofen":       ResolverResult("ibuprofen",       "5640",   "NSAID"),
    "naproxen":        ResolverResult("naproxen",        "7258",   "NSAID"),
    "oxycodone":       ResolverResult("oxycodone",       "7804",   "opioid"),
    "hydrocodone":     ResolverResult("hydrocodone",     "5489",   "opioid"),
    "tramadol":        ResolverResult("tramadol",        "10689",  "opioid"),
    "gabapentin":      ResolverResult("gabapentin",      "25480",  "gabapentinoid"),
    # Antibiotics
    "amoxicillin":     ResolverResult("amoxicillin",     "723",    "penicillin"),
    "azithromycin":    ResolverResult("azithromycin",    "18631",  "macrolide"),
    "ciprofloxacin":   ResolverResult("ciprofloxacin",   "2551",   "fluoroquinolone"),
    "doxycycline":     ResolverResult("doxycycline",     "3640",   "tetracycline"),
    # Brand → generic aliases
    "lipitor":         ResolverResult("atorvastatin",    "83367",  "statin"),
    "crestor":         ResolverResult("rosuvastatin",    "301542", "statin"),
    "norvasc":         ResolverResult("amlodipine",      "17767",  "calcium channel blocker"),
    "ozempic":         ResolverResult("semaglutide",     "1991302","GLP-1 agonist"),
    "zoloft":          ResolverResult("sertraline",      "36437",  "SSRI"),
    "prozac":          ResolverResult("fluoxetine",      "4493",   "SSRI"),
    "lexapro":         ResolverResult("escitalopram",    "321988", "SSRI"),
    "wellbutrin":      ResolverResult("bupropion",       "42347",  "atypical antidepressant"),
    "ativan":          ResolverResult("lorazepam",       "6470",   "benzodiazepine"),
    "xanax":           ResolverResult("alprazolam",      "596",    "benzodiazepine"),
    "prilosec":        ResolverResult("omeprazole",      "7646",   "PPI"),
    "ventolin":        ResolverResult("albuterol",       "435",    "SABA"),
    "proair":          ResolverResult("albuterol",       "435",    "SABA"),
    "tylenol":         ResolverResult("acetaminophen",   "161",    "analgesic"),
    "advil":           ResolverResult("ibuprofen",       "5640",   "NSAID"),
    "motrin":          ResolverResult("ibuprofen",       "5640",   "NSAID"),
    "aleve":           ResolverResult("naproxen",        "7258",   "NSAID"),
    "percocet":        ResolverResult("oxycodone",       "7804",   "opioid"),
    "norco":           ResolverResult("hydrocodone",     "5489",   "opioid"),
    "neurontin":       ResolverResult("gabapentin",      "25480",  "gabapentinoid"),
    "synthroid":       ResolverResult("levothyroxine",   "10582",  "thyroid hormone"),
    "glucophage":      ResolverResult("metformin",       "6809",   "biguanide"),
}


def offline_resolver(name_raw: str) -> ResolverResult | None:
    if not name_raw:
        return None
    key = name_raw.strip().lower()
    # Try exact, then strip salt forms ("losartan potassium" -> "losartan")
    if key in _OFFLINE_DICT:
        return _OFFLINE_DICT[key]
    head = key.split()[0]
    return _OFFLINE_DICT.get(head)


_RESOLVER: Resolver = offline_resolver


def register_resolver(resolver: Resolver) -> None:
    """Replace the default offline resolver (e.g. with a live RxNorm client)."""
    global _RESOLVER
    _RESOLVER = resolver


# ---------------------------------------------------------------------------
# Plausibility rules
# ---------------------------------------------------------------------------

# Plausible dose units per route. Not exhaustive; conservative.
_ROUTE_UNITS: dict[str, set[str]] = {
    "PO":         {"mg", "mcg", "g", "mL", "tablets", "capsules", "units", "IU"},
    "IV":         {"mg", "mcg", "g", "mL", "L", "units", "IU", "mEq", "mg/kg", "mcg/kg"},
    "IM":         {"mg", "mcg", "mL", "units"},
    "SC":         {"mg", "mcg", "mL", "units"},
    "SL":         {"mg", "mcg"},
    "PR":         {"mg", "mL"},
    "topical":    {"%", "g", "mL", "patches"},
    "inhaled":    {"mcg", "puffs", "mg"},
    "nebulized":  {"mg", "mcg", "mL"},
    "intranasal": {"mcg", "sprays", "mL"},
    "ophthalmic": {"drops", "mL", "%"},
    "otic":       {"drops", "mL"},
    "transdermal":{"mg", "mcg", "patches"},
    "vaginal":    {"mg", "g", "mL"},
    "buccal":     {"mg", "mcg"},
}

# Suspiciously high doses by drug class (for an adult). These trigger a warning,
# not a block — pediatric, oncologic, and certain critical-care doses may legitimately exceed.
_HIGH_DOSE_WARN_MG: dict[str, float] = {
    "ACE inhibitor": 80,
    "ARB": 320,
    "beta blocker": 400,
    "statin": 80,
    "SSRI": 200,
    "PPI": 80,
    "biguanide": 2550,
    "SABA": 10,         # mg/day; albuterol is mostly mcg or puffs
    "thiazide diuretic": 50,
    "loop diuretic": 600,
}


# ---------------------------------------------------------------------------
# Validation entry point
# ---------------------------------------------------------------------------

@dataclass
class ValidationResult:
    record: dict
    warnings: list[str]


def validate(records: list[dict]) -> list[ValidationResult]:
    out: list[ValidationResult] = []
    seen_classes: dict[str, list[str]] = {}

    for rec in records:
        warnings: list[str] = []
        rec = dict(rec)  # shallow copy; we add validator_warnings
        name_raw = rec.get("name_raw") or ""
        resolved = _RESOLVER(name_raw)

        if resolved is None:
            rec["name_resolved"] = None
            rec["rxcui"] = None
            rec["unresolved"] = True
            warnings.append(f"name '{name_raw}' did not resolve against drug vocabulary; verify with patient")
        else:
            rec["name_resolved"] = resolved.name
            rec["rxcui"] = resolved.rxcui
            rec["unresolved"] = False
            # Class duplicate detection
            if resolved.drug_class:
                seen_classes.setdefault(resolved.drug_class, []).append(resolved.name)

        # Route/unit plausibility
        unit = rec.get("dose_unit")
        route = rec.get("route")
        if unit and route and route in _ROUTE_UNITS and unit not in _ROUTE_UNITS[route]:
            warnings.append(f"unit '{unit}' is unusual for route '{route}'; verify")

        # High-dose warning for adult-typical thresholds
        try:
            dose = rec.get("dose")
            if (
                resolved is not None
                and resolved.drug_class in _HIGH_DOSE_WARN_MG
                and isinstance(dose, (int, float))
                and unit == "mg"
                and dose > _HIGH_DOSE_WARN_MG[resolved.drug_class]
            ):
                warnings.append(
                    f"dose {dose} mg exceeds typical adult ceiling for {resolved.drug_class}; verify indication"
                )
        except Exception:
            pass

        # Uncertain dose forwarded as warning
        if rec.get("uncertain_dose"):
            warnings.append("dose stated ambiguously; clinician should confirm")

        existing = list(rec.get("validator_warnings") or [])
        rec["validator_warnings"] = existing + warnings
        out.append(ValidationResult(record=rec, warnings=warnings))

    # Same-class duplicate flag (post-loop so we have full picture)
    for cls, names in seen_classes.items():
        if len(names) > 1:
            for r in out:
                if r.record.get("name_resolved") in names:
                    msg = f"same-class duplicate within encounter: {cls} ({', '.join(sorted(set(names)))})"
                    if msg not in r.record["validator_warnings"]:
                        r.record["validator_warnings"].append(msg)
                        r.warnings.append(msg)

    return out


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

if __name__ == "__main__":  # pragma: no cover
    import argparse
    import sys

    p = argparse.ArgumentParser(description="Validate ambient-scribe medication records")
    p.add_argument("--in", dest="path", required=True, help="JSON file containing a list of medication records")
    args = p.parse_args()
    with open(args.path, encoding="utf-8") as f:
        records = json.load(f)
    results = validate(records)
    json.dump(
        [{"record": r.record, "warnings": r.warnings} for r in results],
        sys.stdout,
        indent=2,
    )
    sys.stdout.write("\n")
