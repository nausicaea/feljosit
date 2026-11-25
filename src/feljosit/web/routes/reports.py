"""Reports: visualisations and statistics."""

from __future__ import annotations

from datetime import date
from typing import Annotated

from fastapi import APIRouter, Query, Request
from fastapi.responses import HTMLResponse, Response

from feljosit.money import format_minor
from feljosit.services import reports
from feljosit.services.budget import add_months, month_start
from feljosit.web import charts
from feljosit.web.deps import AppSettings, CurrentAuth
from feljosit.web.rendering import month_short, render

router = APIRouter()

PERIODS = {"3m": 3, "6m": 6, "12m": 12, "24m": 24}


def _axis_formatter(currency: str) -> charts.Formatter:
    """Format axis labels in whole currency units (for example whole francs)."""

    def fmt(value: float) -> str:
        return format_minor(int(value), currency).rsplit(".", 1)[0]

    return fmt


@router.get("/reports", response_class=HTMLResponse)
def reports_page(
    request: Request,
    auth: CurrentAuth,
    settings: AppSettings,
    period: Annotated[str, Query(max_length=3)] = "12m",
) -> Response:
    months = PERIODS.get(period, 12)
    end_month = month_start(date.today())
    start = add_months(end_month, -(months - 1))
    end = add_months(end_month, 1).replace(day=1)
    end = date.fromordinal(end.toordinal() - 1)
    currency = settings.budget_currency

    fmt_axis = _axis_formatter(currency)
    flows = reports.monthly_flows(auth.db, auth.user.id, start, end)
    labels = [month_short(f.month) for f in flows]
    flow_chart = charts.grouped_bars(
        labels,
        [
            ("bar-income", [f.income for f in flows]),
            ("bar-expense", [-f.expenses for f in flows]),
        ],
        fmt_axis,
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
        )
        for cur, series in worth.items()
    }
    return render(
        request,
        "reports.html",
        auth,
        period=period if period in PERIODS else "12m",
        periods=list(PERIODS),
        start=start,
        end=end,
        flows=flows,
        flow_chart=flow_chart,
        spending_rows=spending_rows,
        spending_total=sum(s.total for s in spending),
        summary=reports.summary(flows),
        worth_charts=worth_charts,
    )
