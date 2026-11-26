"""Reports: visualisations and statistics."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import date
from typing import Annotated

from fastapi import APIRouter, Query, Request
from fastapi.responses import HTMLResponse, Response
from pydantic import BeforeValidator

from feljosit.money import format_minor
from feljosit.services import reports
from feljosit.services.budget import add_months, month_start
from feljosit.web import charts
from feljosit.web.deps import AppSettings, Auth, CurrentAuth
from feljosit.web.rendering import month_short, render

router = APIRouter()

PERIODS = {"3m": 3, "6m": 6, "12m": 12, "24m": 24}
ALL_PERIOD = "all"
"""The period from the month of the first transaction to the current month."""
PERIOD_LABELS = {"3m": "3 months", "6m": "6 months", "12m": "12 months", "24m": "24 months"}
PERIOD_LABELS[ALL_PERIOD] = "All"
DEFAULT_PERIOD = "12m"
MAX_MONTHS = 240
"""The maximum length of a custom period (20 years). This limits the size of the charts."""
MIN_DATE = date(1900, 1, 1)
MAX_DATE = date(9998, 12, 31)
"""Later dates are not possible: the month calculations must stay below year 10000."""


def _empty_as_none(value: object) -> object:
    """Change an empty query value to ``None``.

    The date form sends ``from=`` when the user does not type a date.
    """
    return None if value == "" else value


def _last_day_of_month(month: date) -> date:
    return date.fromordinal(add_months(month, 1).toordinal() - 1)


def _preset_range(period: str, today: date) -> tuple[date, date]:
    """The range of a preset period: the last full months, including the current month."""
    months = PERIODS.get(period, PERIODS[DEFAULT_PERIOD])
    end_month = month_start(today)
    return add_months(end_month, -(months - 1)), _last_day_of_month(end_month)


def all_range(data_range: tuple[date, date] | None, today: date) -> tuple[date, date, bool]:
    """The range of the "All" period and ``True`` if the range was made shorter.

    The range starts in the month of the first transaction. It stops at the end of the
    current month, or at the end of the month of the last transaction if that is later.
    If the range is longer than ``MAX_MONTHS``, only the last ``MAX_MONTHS`` months are
    used. If there are no transactions, the range is the current month.
    """
    end_month = month_start(today)
    if data_range is None:
        return end_month, _last_day_of_month(end_month), False
    first, last = data_range
    end_month = max(end_month, month_start(min(last, MAX_DATE)))
    start = max(month_start(first), month_start(MIN_DATE))
    earliest = add_months(end_month, -(MAX_MONTHS - 1))
    shortened = start < earliest
    return max(start, earliest), _last_day_of_month(end_month), shortened


def custom_range(start: date | None, end: date | None, today: date) -> tuple[date, date]:
    """The range of a custom period. Raise ``ValueError`` if the range is not valid.

    If there is no end date, the end is today. If there is no start date, the start
    is the first day of the month 11 months before the end month.
    """
    if end is None:
        end = today
    if not MIN_DATE <= end <= MAX_DATE or not (start is None or MIN_DATE <= start <= MAX_DATE):
        raise ValueError(f"Type dates from {MIN_DATE:%d.%m.%Y} to {MAX_DATE:%d.%m.%Y}.")
    if start is None:
        start = max(add_months(month_start(end), -11), MIN_DATE)
    if start > end:
        raise ValueError("The start date must be on or before the end date.")
    months = (end.year - start.year) * 12 + end.month - start.month + 1
    if months > MAX_MONTHS:
        raise ValueError(f"The period must be a maximum of {MAX_MONTHS // 12} years.")
    return start, end


OptionalDate = Annotated[date | None, BeforeValidator(_empty_as_none)]


def _month_labels(months: list[date]) -> tuple[int, int]:
    """The label step and offset for a chart with one value for each month.

    The labels are on calendar boundaries, for example January, April, July and
    October for a step of 3 months.
    """
    step = charts.month_label_step(len(months))
    if not months:
        return step, 0
    return step, months[0].year * 12 + months[0].month - 1


def _axis_formatter(currency: str) -> charts.Formatter:
    """Format axis labels in whole currency units (for example whole francs)."""

    def fmt(value: float) -> str:
        return format_minor(int(value), currency).rsplit(".", 1)[0]

    return fmt


@dataclass(frozen=True)
class ReportPeriod:
    """The period of a report page, from the query parameters."""

    start: date
    end: date
    #: The preset period, or ``None`` for a custom period.
    period: str | None
    error: str | None
    shortened: bool
    form_from: str
    form_to: str

    @property
    def query(self) -> dict[str, str]:
        """The query parameters that select this period on a different report page."""
        if self.period is None:
            return {"from": self.start.isoformat(), "to": self.end.isoformat()}
        return {"period": self.period}

    def template_context(self) -> dict[str, object]:
        return {
            "period": self.period,
            "periods": PERIOD_LABELS,
            "custom": self.period is None,
            "error": self.error,
            "shortened": self.shortened,
            "max_years": MAX_MONTHS // 12,
            "form_from": self.form_from,
            "form_to": self.form_to,
            "start": self.start,
            "end": self.end,
            "period_query": self.query,
        }


def resolve_period(
    auth: Auth, period: str, date_from: date | None, date_to: date | None
) -> ReportPeriod:
    today = date.today()
    error: str | None = None
    custom = date_from is not None or date_to is not None
    shortened = False
    if period not in PERIOD_LABELS:
        period = DEFAULT_PERIOD
    if custom:
        try:
            start, end = custom_range(date_from, date_to, today)
        except ValueError as exc:
            error = str(exc)
            custom = False
    if not custom and period == ALL_PERIOD:
        data_range = reports.transaction_date_range(auth.db, auth.user.id)
        start, end, shortened = all_range(data_range, today)
    elif not custom:
        start, end = _preset_range(period, today)
    return ReportPeriod(
        start=start,
        end=end,
        period=None if custom else period,
        error=error,
        shortened=shortened,
        form_from=date_from.isoformat() if date_from else start.isoformat(),
        form_to=date_to.isoformat() if date_to else end.isoformat(),
    )


PeriodQuery = Annotated[str, Query(max_length=3)]
DateFrom = Annotated[OptionalDate, Query(alias="from")]
DateTo = Annotated[OptionalDate, Query(alias="to")]


@router.get("/reports", response_class=HTMLResponse)
def reports_page(
    request: Request,
    auth: CurrentAuth,
    settings: AppSettings,
    period: PeriodQuery = DEFAULT_PERIOD,
    date_from: DateFrom = None,
    date_to: DateTo = None,
) -> Response:
    chosen = resolve_period(auth, period, date_from, date_to)
    start, end = chosen.start, chosen.end
    currency = settings.budget_currency

    fmt_axis = _axis_formatter(currency)
    flows = reports.monthly_flows(auth.db, auth.user.id, start, end)
    labels = [month_short(f.month) for f in flows]
    label_step, label_offset = _month_labels([f.month for f in flows])
    flow_chart = charts.grouped_bars(
        labels,
        [
            ("bar-income", [f.income for f in flows]),
            ("bar-expense", [-f.expenses for f in flows]),
        ],
        fmt_axis,
        label_step,
        label_offset,
    )
    spending = reports.spending_by_category(auth.db, auth.user.id, start, end)
    spending_rows = charts.horizontal_bars(
        [(s.name, s.total) for s in spending[:15]],
        lambda v: format_minor(int(v), currency),
    )
    worth = reports.net_worth(auth.db, auth.user.id, start, end)
    worth_charts = {
        cur: charts.line(
            [month_short(m) for m, _ in series],
            [v for _, v in series],
            _axis_formatter(cur),
            label_step,
            label_offset,
        )
        for cur, series in worth.items()
    }
    return render(
        request,
        "reports.html",
        auth,
        **chosen.template_context(),
        keep={},
        flows=flows,
        flow_chart=flow_chart,
        spending_rows=spending_rows,
        spending_total=sum(s.total for s in spending),
        summary=reports.summary(flows),
        worth_charts=worth_charts,
        status_code=400 if chosen.error else 200,
    )


SORTS = {"budget": "Budget order", "total": "Largest total", "change": "Largest increase"}
COUNTERPARTY_SORTS = {"total": "Largest total", "change": "Largest increase"}


def _bucket_label(start: date, size: int) -> str:
    if size == 12:
        return str(start.year)
    if size == 3:
        return f"Q{(start.month - 1) // 3 + 1} {start.year % 100:02d}"
    return month_short(start)


HEAT_STEPS = (0.10, 0.25, 0.50, 1.00)
"""A cell gets a heat level if it is this much above the average of its row (+10 %, +25 % …)."""


def _heat(value: int, average: int) -> int:
    """A heat level from 0 to 4: how much a cell is above the average of its row."""
    if average <= 0 or value <= average:
        return 0
    above = value / average - 1
    return sum(1 for step in HEAT_STEPS if above >= step)


def _sorted_rows(
    rows: tuple[reports.SpendingSeries, ...], sort: str
) -> list[reports.SpendingSeries]:
    if sort == "total":
        return sorted(rows, key=lambda r: r.total, reverse=True)
    if sort == "change":
        return sorted(
            rows,
            key=lambda r: (r.change_percent is None, -(r.change_percent or 0.0)),
        )
    return list(rows)


@router.get("/reports/categories", response_class=HTMLResponse)
def category_report_page(
    request: Request,
    auth: CurrentAuth,
    settings: AppSettings,
    period: PeriodQuery = DEFAULT_PERIOD,
    date_from: DateFrom = None,
    date_to: DateTo = None,
    sort: Annotated[str, Query(max_length=10)] = "budget",
    category: Annotated[str, Query(max_length=20)] = "",
) -> Response:
    chosen = resolve_period(auth, period, date_from, date_to)
    currency = settings.budget_currency
    if sort not in SORTS:
        sort = "budget"
    data = reports.compare_categories(auth.db, auth.user.id, chosen.start, chosen.end)

    selected = next((r for r in data.rows if category and r.key == category), None)
    detail: dict[str, object] | None = None
    if selected is not None:
        detail = {
            "row": selected,
            "chart": _month_chart(data.months, selected.monthly, currency),
            "counterparties": reports.top_counterparties(
                auth.db, auth.user.id, selected.category_id, chosen.start, chosen.end
            ),
        }
    return render(
        request,
        "report_categories.html",
        auth,
        **chosen.template_context(),
        data=data,
        rows=_sorted_rows(data.rows, sort),
        sort=sort,
        sorts=SORTS,
        bucket_labels=[_bucket_label(b, data.bucket_months) for b in data.buckets],
        heat=_heat,
        keep={"sort": sort} | ({"category": category} if detail else {}),
        detail=detail,
        status_code=400 if chosen.error else 200,
    )


def _month_chart(months: tuple[date, ...], monthly: tuple[int, ...], currency: str) -> charts.Chart:
    label_step, label_offset = _month_labels(list(months))
    return charts.grouped_bars(
        [month_short(m) for m in months],
        [("bar-expense", list(monthly))],
        _axis_formatter(currency),
        label_step,
        label_offset,
    )


@router.get("/reports/counterparties", response_class=HTMLResponse)
def counterparty_report_page(
    request: Request,
    auth: CurrentAuth,
    settings: AppSettings,
    period: PeriodQuery = DEFAULT_PERIOD,
    date_from: DateFrom = None,
    date_to: DateTo = None,
    sort: Annotated[str, Query(max_length=10)] = "total",
    counterparty: Annotated[str, Query(max_length=140)] = "",
) -> Response:
    chosen = resolve_period(auth, period, date_from, date_to)
    if sort not in COUNTERPARTY_SORTS:
        sort = "total"
    data = reports.compare_counterparties(auth.db, auth.user.id, chosen.start, chosen.end)
    detail: dict[str, object] | None = None
    if counterparty:
        # The table shows only the largest counterparties. A link from a different
        # page can select any counterparty.
        selected = next((r for r in data.rows if r.key == counterparty), None) or (
            reports.counterparty_series(
                auth.db, auth.user.id, counterparty, chosen.start, chosen.end
            )
        )
        if selected is not None:
            detail = {
                "row": selected,
                "chart": _month_chart(data.months, selected.monthly, settings.budget_currency),
                "categories": reports.counterparty_categories(
                    auth.db, auth.user.id, counterparty, chosen.start, chosen.end
                ),
            }
    return render(
        request,
        "report_counterparties.html",
        auth,
        **chosen.template_context(),
        data=data,
        rows=_sorted_rows(data.rows, sort),
        sort=sort,
        sorts=COUNTERPARTY_SORTS,
        bucket_labels=[_bucket_label(b, data.bucket_months) for b in data.buckets],
        heat=_heat,
        detail=detail,
        keep={"sort": sort} | ({"counterparty": counterparty} if detail else {}),
        status_code=400 if chosen.error else 200,
    )
