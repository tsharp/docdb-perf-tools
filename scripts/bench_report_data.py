#!/usr/bin/env python3

import json
import re
from pathlib import Path


def _get_value(value, *path):
    for part in path:
        if not isinstance(value, dict):
            return None
        value = value.get(part)
    return value


def _first_value(*values):
    return next((value for value in values if value is not None and value != ""), None)


def _format_document_size(size):
    if size is None:
        return "unknown"

    size = int(size)
    for suffix, divisor in (("gb", 1024**3), ("mb", 1024**2), ("kb", 1024)):
        if size >= divisor and size % divisor == 0:
            return f"{size // divisor}{suffix}"
    return f"{size}b"


def _users_from_path(report_file):
    for part in report_file.parts:
        match = re.fullmatch(r"(\d+)_users", part)
        if match:
            return int(match.group(1))
    return None


def load_bench_results(results_dir):
    results_path = Path(results_dir).resolve()
    if not results_path.is_dir():
        raise FileNotFoundError(f"Benchmark results directory not found: {results_path}")

    report_files = sorted(results_path.rglob("*_report.json"))
    if not report_files:
        raise FileNotFoundError(f"No benchmark report JSON files found under: {results_path}")

    rows = []
    for report_file in report_files:
        with report_file.open(encoding="utf-8") as stream:
            report = json.load(stream)

        metadata = report.get("metadata", {})
        properties = metadata.get("properties", {})
        summary = report.get("summary", {})
        writes = report.get("writes", {})
        latency = _first_value(summary.get("percentiles_ms"), writes.get("latency_ms"), {})

        users = _first_value(metadata.get("workers"), _users_from_path(report_file))
        cluster_size = properties.get("cluster.size")
        cluster_type = properties.get("cluster.type")
        if users is None or cluster_size is None or cluster_type is None:
            raise ValueError(
                f"Missing users, cluster.size, or cluster.type in benchmark report: {report_file}"
            )

        document_size = _first_value(
            properties.get("doc.size"),
            _get_value(metadata, "workload_params", "document_size"),
        )
        request_count = _first_value(
            summary.get("total_requests"),
            writes.get("total_operations"),
        )
        failure_count = _first_value(
            summary.get("total_failures"),
            writes.get("total_failures"),
            0,
        )
        requests_per_second = _first_value(
            summary.get("requests_per_sec"),
            writes.get("operations_per_sec"),
        )
        failures_per_second = _first_value(
            summary.get("failures_per_sec"),
            writes.get("failures_per_sec"),
            0.0,
        )

        required_values = {
            "request count": request_count,
            "requests per second": requests_per_second,
            "average latency": _first_value(
                summary.get("avg_response_time_ms"), latency.get("avg")
            ),
            "minimum latency": _first_value(
                summary.get("min_response_time_ms"), latency.get("min")
            ),
            "maximum latency": _first_value(
                summary.get("max_response_time_ms"), latency.get("max")
            ),
            "p50 latency": latency.get("p50"),
            "p75 latency": latency.get("p75"),
            "p90 latency": latency.get("p90"),
            "p95 latency": latency.get("p95"),
            "p99 latency": latency.get("p99"),
        }
        missing = [name for name, value in required_values.items() if value is None]
        if missing:
            raise ValueError(
                f"Missing {', '.join(missing)} in benchmark report: {report_file}"
            )

        rows.append(
            {
                "Cluster Size": str(cluster_size),
                "Cluster Type": str(cluster_type),
                "Driver Language": str(metadata.get("driver_language") or "unknown"),
                "Driver": str(metadata.get("driver") or "unknown"),
                "Driver Version": str(metadata.get("driver_version") or "unknown"),
                "Commit": str(
                    _first_value(
                        properties.get("commit"),
                        properties.get("cluster.version"),
                        metadata.get("commit"),
                        "unknown",
                    )
                ),
                "Users": str(users),
                "Document Size": _format_document_size(document_size),
                "Test": str(
                    _first_value(
                        metadata.get("run_label"),
                        metadata.get("benchmark_name"),
                        report_file.parent.name,
                    )
                ),
                "Request Count": request_count,
                "Failure Count": failure_count,
                "Median Response Time": required_values["p50 latency"],
                "Average Response Time": required_values["average latency"],
                "Min Response Time": required_values["minimum latency"],
                "Max Response Time": required_values["maximum latency"],
                "Requests/s": requests_per_second,
                "Failures/s": failures_per_second,
                "50%": required_values["p50 latency"],
                "75%": required_values["p75 latency"],
                "90%": required_values["p90 latency"],
                "95%": required_values["p95 latency"],
                "99%": required_values["p99 latency"],
            }
        )

    return rows, report_files
