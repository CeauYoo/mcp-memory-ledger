"""Small falsifiable prose checks; never a general semantic-quality certificate."""
from datetime import date
import re

def check_answer(answer,rules):
    if not rules.get('planning_start'):return {}
    start=date.fromisoformat(rules['planning_start'])
    # Dedicated synthetic S16 rule: preparation phases may not precede its stated
    # start. All calendar dates in this task describe preparation/delivery, not
    # historical analysis. Other scenarios are deliberately outside this check.
    found=[];valid=True
    for year,month,day in re.findall(r'(?<!\d)(\d{4})[-年](\d{1,2})[-月](\d{1,2})(?:日)?(?!\d)',answer):
        try:found.append(date(int(year),int(month),int(day)))
        except ValueError:valid=False
    return {'planning_dates_parseable':valid,
            'no_explicit_date_before_planning_start':all(value>=start for value in found)}
