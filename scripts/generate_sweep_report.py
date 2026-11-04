#!/usr/bin/env python3
"""
Generate user sweep performance report from CSV data.
Creates line charts showing performance curves by user count,
with separate lines for test, cluster type, and driver metadata,
and separate charts for each SKU (cluster size).
"""

import argparse
import csv
import json
import re
from collections import defaultdict
from datetime import datetime
from statistics import mean
from glob import glob
from pathlib import Path

from bench_report_data import load_bench_results
from generate_user_sweep_report import (
    generate_html as generate_user_sweep_html,
    generate_user_sweep_charts as generate_user_count_sweep_charts,
)

def normalize_sku(cluster_size):
    """Return a lowercase SKU label such as m50 for either latest or legacy values."""
    sku = str(cluster_size).strip().lower()
    return sku if sku.startswith('m') else f'm{sku}'

def sku_sort_key(sku):
    """Sort m10, m50, m200 numerically while tolerating non-standard labels."""
    match = re.search(r'\d+', str(sku))
    return int(match.group(0)) if match else str(sku)

def display_sku(sku):
    """Return an uppercase SKU label for report titles."""
    return normalize_sku(sku).upper()

def document_size_sort_key(document_size):
    """Sort document sizes such as 256b, 64kb, and 1mb by byte size."""
    match = re.match(r'\s*(\d+(?:\.\d+)?)\s*([kmgt]?b)?\s*$', str(document_size), re.IGNORECASE)
    if not match:
        return (float('inf'), str(document_size))

    value = float(match.group(1))
    unit = (match.group(2) or 'b').lower()
    multipliers = {
        'b': 1,
        'kb': 1024,
        'mb': 1024 ** 2,
        'gb': 1024 ** 3,
        'tb': 1024 ** 4,
    }
    return (value * multipliers.get(unit, 1), str(document_size))

def format_document_size(document_size):
    """Return a compact display label for a document size value."""
    return str(document_size).strip()

def normalize_commit(commit):
    """Return a stable commit value or unknown when the CSV does not provide one."""
    normalized = str(commit).strip()
    return normalized if normalized and normalized.lower() != 'nan' else 'unknown'

def format_commit_label(commit):
    """Return a compact commit label segment for chart legends."""
    normalized = normalize_commit(commit)
    return '' if normalized == 'unknown' else f', commit {normalized[:12]}'

def chart_color_pair(hex_color):
    """Convert a readable hex color into Chart.js border/background colors."""
    hex_color = hex_color.lstrip('#')
    red = int(hex_color[0:2], 16)
    green = int(hex_color[2:4], 16)
    blue = int(hex_color[4:6], 16)
    return {
        'border': f'rgba({red}, {green}, {blue}, 1)',
        'bg': f'rgba({red}, {green}, {blue}, 0.18)'
    }

def read_and_process_csv(csv_pattern=None, rows=None):
    """Read CSV files and average similar test runs."""
    data = list(rows or [])

    if rows is None:
        csv_files = sorted(glob(csv_pattern))
        if not csv_files:
            raise FileNotFoundError(f"No CSV files found matching pattern: {csv_pattern}")

        print(f"Found {len(csv_files)} sweep files: {csv_files}")

        for csv_file in csv_files:
            with open(csv_file, 'r', encoding='utf-8') as f:
                reader = csv.DictReader(f)
                data.extend(reader)
            print(f"  Loaded rows from {csv_file}")

    for row in data:
        row['SKU'] = normalize_sku(row['Cluster Size'])
        for column in ['Driver Language', 'Driver', 'Driver Version']:
            row.setdefault(column, 'unknown')
        commit_column = next((column for column in row if column.lower() == 'commit'), None)
        row['Commit'] = normalize_commit(row.get(commit_column, 'unknown')) if commit_column else 'unknown'
    
    print(f"Combined total: {len(data)} rows")
    
    # Group by SKU, cluster type, driver metadata, user count, document size, and test.
    grouped = defaultdict(list)
    
    for row in data:
        key = (
            row['SKU'],
            row['Cluster Type'],
            row['Driver Language'],
            row['Driver'],
            row['Driver Version'],
            row['Commit'],
            row['Users'],
            row['Document Size'],
            row['Test']
        )
        grouped[key].append(row)
    
    # Average the grouped data
    averaged_data = []
    
    for key, rows in grouped.items():
        sku, cluster_type, driver_language, driver, driver_version, commit, users, document_size, test = key
        
        # Calculate averages for numeric fields
        avg_row = {
            'SKU': sku,
            'Cluster Size': sku,
            'Cluster Type': cluster_type,
            'Driver Language': driver_language,
            'Driver': driver,
            'Driver Version': driver_version,
            'Commit': commit,
            'Users': users,
            'Document Size': document_size,
            'Test': test,
            'Run Count': len(rows),
            'Request Count': mean(float(r['Request Count']) for r in rows),
            'Failure Count': mean(float(r['Failure Count']) for r in rows),
            'Median Response Time': mean(float(r['Median Response Time']) for r in rows),
            'Average Response Time': mean(float(r['Average Response Time']) for r in rows),
            'Min Response Time': mean(float(r['Min Response Time']) for r in rows),
            'Max Response Time': mean(float(r['Max Response Time']) for r in rows),
            'Requests/s': mean(float(r['Requests/s']) for r in rows),
            'Failures/s': mean(float(r['Failures/s']) for r in rows),
            '50%': mean(float(r['50%']) for r in rows),
            '75%': mean(float(r['75%']) for r in rows),
            '90%': mean(float(r['90%']) for r in rows),
            '95%': mean(float(r['95%']) for r in rows),
            '99%': mean(float(r['99%']) for r in rows),
        }
        
        averaged_data.append(avg_row)
    
    return averaged_data

def generate_user_sweep_charts(data):
    """Generate line chart configurations for user sweep analysis."""
    chart_sections = []
    
    # Function to extract test category (e.g., "insert_bench" from "insert_bench_1kb")
    def get_test_category(test_name):
        """Extract the base category from test name (insert_bench, point_read, etc.)"""
        if 'insert_bench' in test_name:
            return 'insert_bench'
        elif 'point_read' in test_name:
            return 'point_read'
        else:
            # For other tests, just return the test name as-is
            return test_name
    
    # Add test category to data
    for row in data:
        row['Test Category'] = get_test_category(row['Test'])
    
    readable_colors = [
        '#2563eb', '#dc2626', '#059669', '#d97706', '#7c3aed', '#0891b2',
        '#be123c', '#65a30d', '#c2410c', '#4338ca', '#0f766e', '#a21caf',
        '#ca8a04', '#0284c7', '#b91c1c', '#15803d', '#9333ea', '#ea580c'
    ]
    
    # Group data by (cluster size, test category)
    by_sku_category = defaultdict(list)
    for row in data:
        key = (row['SKU'], row['Test Category'])
        by_sku_category[key].append(row)
    
    # Create charts for each SKU + test category combination
    for (sku, test_category) in sorted(by_sku_category.keys(), key=lambda x: (sku_sort_key(x[0]), x[1])):
        category_data = by_sku_category[(sku, test_category)]
        
        # Get all unique document sizes for this SKU and test category (sorted by byte size)
        document_sizes = sorted(set(row['Document Size'] for row in category_data), key=document_size_sort_key)
        labels = [format_document_size(size) for size in document_sizes]
        
        # Group by test, user count, cluster type, and driver metadata to create one line per combination.
        series_map = defaultdict(list)
        
        for row in category_data:
            key = (row['Test'], row['Users'], row['Cluster Type'], row['Driver Language'], row['Driver'], row['Driver Version'], row['Commit'])
            series_map[key].append(row)
        
        # Build datasets for Requests/s chart
        rps_datasets = []
        # Build datasets for Response Time chart
        response_datasets = []
        
        chart_color_offset = sum(ord(char) for char in f'{sku}:{test_category}') % len(readable_colors)
        
        for series_index, ((test_name, users, cluster_type, driver_language, driver, driver_version, commit), series_rows) in enumerate(sorted(series_map.items())):
            # Create a lookup by document size
            data_by_document_size = {r['Document Size']: r for r in series_rows}
            
            # Build data arrays aligned with document_sizes
            rps_values = [data_by_document_size[size]['Requests/s'] if size in data_by_document_size else None 
                          for size in document_sizes]
            
            color = chart_color_pair(readable_colors[(chart_color_offset + series_index) % len(readable_colors)])
            label = f'{test_name}, {users} users ({cluster_type}, {driver_language} {driver} {driver_version}{format_commit_label(commit)})'
            
            # Add dataset for RPS
            rps_datasets.append({
                'label': label,
                'data': rps_values,
                'borderColor': color['border'],
                'backgroundColor': color['bg'],
                'borderWidth': 2,
                'fill': False,
                'tension': 0.1,
                'pointRadius': 4,
                'pointHoverRadius': 6
            })
            
            # Add datasets for percentile response times
            for percentile_label, percentile_column, border_dash in [
                ('p50', '50%', []),
                ('p95', '95%', [6, 4]),
                ('p99', '99%', [2, 4]),
            ]:
                response_datasets.append({
                    'label': f'{label} {percentile_label}',
                    'data': [data_by_document_size[size][percentile_column] if size in data_by_document_size else None
                             for size in document_sizes],
                    'borderColor': color['border'],
                    'backgroundColor': color['bg'],
                    'borderWidth': 2,
                    'borderDash': border_dash,
                    'fill': False,
                    'tension': 0.1,
                    'pointRadius': 4,
                    'pointHoverRadius': 6
                })
        
        # Create chart section for this SKU + test category
        chart_id_base = f"sku_{sku}_test_{test_category}".replace('-', '_').replace(' ', '_')
        
        section = {
            'sku': sku,
            'test_type': test_category,
            'charts': [
                {
                    'id': f'chart_rps_{chart_id_base}',
                    'title': f'Throughput by Document Size - {test_category} (Cluster Size: {display_sku(sku)})',
                    'labels': labels,
                    'datasets': rps_datasets,
                    'type': 'rps',
                    'details': category_data
                },
                {
                    'id': f'chart_response_{chart_id_base}',
                    'title': f'Latency Percentiles by Document Size - {test_category} (Cluster Size: {display_sku(sku)})',
                    'labels': labels,
                    'datasets': response_datasets,
                    'type': 'response',
                    'details': category_data
                }
            ]
        }
        
        chart_sections.append(section)
    
    return chart_sections

def generate_html(data, chart_sections, output_file):
    """Generate HTML file with line charts and data table."""
    output_path = Path(output_file)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    generated_at = datetime.now().astimezone().strftime('%Y-%m-%d %H:%M:%S %Z')
    
    html = f"""<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Document Size Sweep Performance Report</title>
    <script src="https://cdn.jsdelivr.net/npm/chart.js"></script>
    <style>
        body {{
            font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif;
            margin: 0;
            padding: 20px;
            background-color: #f5f5f5;
        }}
        
        .container {{
            max-width: 85%;
            margin: 0 auto;
            background-color: white;
            padding: 30px;
            border-radius: 8px;
            box-shadow: 0 2px 4px rgba(0,0,0,0.1);
        }}
        
        h1 {{
            color: #333;
            border-bottom: 3px solid #4CAF50;
            padding-bottom: 10px;
            margin-bottom: 30px;
        }}
        
        h2 {{
            color: #555;
            margin-top: 40px;
            margin-bottom: 20px;
        }}
        
        h3 {{
            color: #333;
            margin-top: 50px;
            border-bottom: 2px solid #ddd;
            padding-bottom: 10px;
        }}
        
        .charts-grid {{
            display: grid;
            grid-template-columns: 1fr 1fr;
            gap: 30px;
            margin-bottom: 50px;
        }}
        
        .chart-container {{
            position: relative;
            padding: 20px;
            background-color: #fafafa;
            border-radius: 8px;
            border: 1px solid #e0e0e0;
        }}
        
        .chart-wrapper {{
            position: relative;
            height: 500px;
            margin-bottom: 20px;
        }}
        
        /* Smaller charts in 4-column layout */
        .charts-grid-4col .chart-wrapper {{
            height: 400px;
        }}
        
        .charts-grid-4col .chart-container {{
            padding: 15px;
        }}
        
        .charts-grid-4col h5 {{
            font-size: 14px;
            margin: 0 0 10px 0;
        }}
        
        @media (max-width: 1200px) {{
            .charts-grid {{
                grid-template-columns: 1fr;
            }}
        }}
        
        table {{
            width: 100%;
            border-collapse: collapse;
            margin-top: 20px;
            font-size: 11px;
        }}
        
        th, td {{
            padding: 8px;
            text-align: left;
            border-bottom: 1px solid #ddd;
        }}
        
        th {{
            background-color: #4CAF50;
            color: white;
            font-weight: bold;
        }}
        
        tr:hover {{
            background-color: #f5f5f5;
        }}
        
        .metric {{
            text-align: right;
            font-family: 'Courier New', monospace;
        }}
        
        .summary {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 15px;
            margin-bottom: 30px;
        }}
        
        .summary-card {{
            background-color: #e3f2fd;
            padding: 15px;
            border-radius: 8px;
            border-left: 4px solid #2196F3;
        }}
        
        .summary-card h3 {{
            margin: 0 0 5px 0;
            font-size: 14px;
            color: #666;
        }}
        
        .summary-card .value {{
            font-size: 24px;
            font-weight: bold;
            color: #333;
        }}
        
        /* Tab styles */
        .tabs {{
            display: flex;
            border-bottom: 2px solid #4CAF50;
            margin-top: 30px;
            margin-bottom: 0;
            gap: 5px;
        }}
        
        .tab-button {{
            background-color: #f0f0f0;
            border: none;
            padding: 12px 24px;
            cursor: pointer;
            font-size: 16px;
            font-weight: 500;
            transition: all 0.3s;
            border-top-left-radius: 8px;
            border-top-right-radius: 8px;
            color: #666;
        }}
        
        .tab-button:hover {{
            background-color: #e0e0e0;
        }}
        
        .tab-button.active {{
            background-color: #4CAF50;
            color: white;
        }}
        
        .tab-content {{
            display: none;
            padding: 30px 0;
        }}
        
        .tab-content.active {{
            display: block;
        }}
        
        /* 4-column grid for SKU overview */
        .charts-grid-4col {{
            display: grid;
            grid-template-columns: repeat(4, 1fr);
            gap: 20px;
            margin-bottom: 50px;
        }}
        
        @media (max-width: 1600px) {{
            .charts-grid-4col {{
                grid-template-columns: repeat(2, 1fr);
            }}
        }}
        
        @media (max-width: 1200px) {{
            .charts-grid-4col {{
                grid-template-columns: 1fr;
            }}
        }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Document Size Sweep Performance Report</h1>
        <p>Generated: {generated_at}</p>
        
        <div class="summary">
            <div class="summary-card">
                <h3>Total Test Configs</h3>
                <div class="value">{len(data)}</div>
            </div>
            <div class="summary-card">
                <h3>Test Types</h3>
                <div class="value">{len(set(r['Test'] for r in data))}</div>
            </div>
            <div class="summary-card">
                <h3>Cluster Types</h3>
                <div class="value">{len(set(r['Cluster Type'] for r in data))}</div>
            </div>
            <div class="summary-card">
                <h3>Cluster Sizes (SKUs)</h3>
                <div class="value">{len(set(r['Cluster Size'] for r in data))}</div>
            </div>
            <div class="summary-card">
                <h3>Document Sizes</h3>
                <div class="value">{len(set(r['Document Size'] for r in data))}</div>
            </div>
            <div class="summary-card">
                <h3>Commits</h3>
                <div class="value">{len(set(r['Commit'] for r in data if r.get('Commit') != 'unknown'))}</div>
            </div>
        </div>
        
        <h2>Performance Curves by Document Size</h2>
        <p style="color: #666; font-style: italic;">
            Each chart shows how performance changes with document size. 
            Use the "By SKU" tab to see all tests for each cluster size, or click test type tabs to compare across SKUs.
        </p>
        
        <!-- Tabs -->
        <div class="tabs">
            <button class="tab-button active" onclick="openTab(event, 'by-sku')">By SKU</button>
"""
    
    # Get unique test types
    test_types = sorted(set(section['test_type'] for section in chart_sections))
    
    for test_type in test_types:
        html += f"""
            <button class="tab-button" onclick="openTab(event, '{test_type}')">{test_type}</button>
"""
    
    html += """
        </div>
"""
    
    # Group sections by test type and by SKU
    from collections import defaultdict
    sections_by_test = defaultdict(list)
    sections_by_sku = defaultdict(list)
    for section in chart_sections:
        sections_by_test[section['test_type']].append(section)
        sections_by_sku[section['sku']].append(section)
    
    # Add "By SKU" tab content first (active by default)
    html += """
        <div id="by-sku" class="tab-content active">
            <h3 style="margin-top: 0;">All Tests by Cluster Size (SKU)</h3>
"""
    
    for sku in sorted(sections_by_sku.keys(), key=sku_sort_key):
        html += f"""
            <h4>Cluster Size {display_sku(sku)}</h4>
            <div class="charts-grid-4col">
"""
        
        for section in sections_by_sku[sku]:
            for chart in section['charts']:
                html += f"""
                <div class="chart-container">
                    <h5 style="margin-top: 0;">{chart['title'].replace(f'(Cluster Size: {display_sku(sku)})', '').strip()}</h5>
                    <div class="chart-wrapper">
                        <canvas id="{chart['id']}"></canvas>
                    </div>
                </div>
"""
        
        html += """
            </div>
"""
    
    html += """
        </div>
"""
    
    # Add tab content for each test type
    for i, test_type in enumerate(test_types):
        html += f"""
        <div id="{test_type}" class="tab-content">
            <h3 style="margin-top: 0;">Test Type: {test_type}</h3>
"""
        
        for section in sections_by_test[test_type]:
            html += f"""
            <h4>Cluster Size {display_sku(section['sku'])}</h4>
            
            <div class="charts-grid">
"""
            
            for chart in section['charts']:
                # Create unique canvas ID for test type tabs
                tab_chart_id = f"{chart['id']}_tab_{test_type.replace('_', '')}"
                html += f"""
                <div class="chart-container">
                    <h5 style="margin-top: 0;">{chart['title']}</h5>
                    <div class="chart-wrapper">
                        <canvas id="{tab_chart_id}"></canvas>
                    </div>
                </div>
"""
            
            html += """
            </div>
"""
        
        html += """
        </div>
"""
    
    # Add full data table
    html += """
        <h2>Complete Data Table</h2>
        <table>
            <thead>
                <tr>
                    <th>Cluster Size</th>
                    <th>Type</th>
                    <th>Driver Language</th>
                    <th>Driver</th>
                    <th>Driver Version</th>
                    <th>Commit</th>
                    <th>Test</th>
                    <th class="metric">Users</th>
                    <th class="metric">Document Size</th>
                    <th class="metric">Runs</th>
                    <th class="metric">Requests/s</th>
                    <th class="metric">Avg Response (ms)</th>
                    <th class="metric">Median (ms)</th>
                    <th class="metric">95th %ile</th>
                    <th class="metric">99th %ile</th>
                </tr>
            </thead>
            <tbody>
"""
    
    for row in sorted(data, key=lambda x: (sku_sort_key(x['SKU']), x['Test'], x['Cluster Type'], x['Driver Language'], x['Driver'], x['Driver Version'], x['Commit'], int(x['Users']), document_size_sort_key(x['Document Size']))):
        html += f"""
                <tr>
                    <td>{display_sku(row['SKU'])}</td>
                    <td>{row['Cluster Type']}</td>
                    <td>{row['Driver Language']}</td>
                    <td>{row['Driver']}</td>
                    <td>{row['Driver Version']}</td>
                    <td>{row['Commit']}</td>
                    <td>{row['Test']}</td>
                    <td class="metric">{row['Users']}</td>
                    <td class="metric">{row['Document Size']}</td>
                    <td class="metric">{row['Run Count']}</td>
                    <td class="metric">{row['Requests/s']:.2f}</td>
                    <td class="metric">{row['Average Response Time']:.2f}</td>
                    <td class="metric">{row['Median Response Time']:.2f}</td>
                    <td class="metric">{row['95%']:.2f}</td>
                    <td class="metric">{row['99%']:.2f}</td>
                </tr>
"""
    
    html += """
            </tbody>
        </table>
    </div>
    
    <script>
"""
    
    # Get unique test types for tab chart creation
    test_types = sorted(set(section['test_type'] for section in chart_sections))
    
    # Add chart initialization scripts
    for section in chart_sections:
        for chart in section['charts']:
            chart_type = 'line'
            y_label = 'Requests/s' if chart.get('type') == 'rps' else 'Latency (ms)'
            
            # Chart configuration
            chart_config = f"""{{
            type: '{chart_type}',
            data: {{
                labels: {json.dumps(chart['labels'])},
                datasets: {json.dumps(chart['datasets'])}
            }},
            options: {{
                responsive: true,
                maintainAspectRatio: false,
                plugins: {{
                    legend: {{
                        display: true,
                        position: 'top'
                    }},
                    title: {{
                        display: false
                    }},
                    tooltip: {{
                        mode: 'index',
                        intersect: false
                    }}
                }},
                interaction: {{
                    mode: 'nearest',
                    axis: 'x',
                    intersect: false
                }},
                scales: {{
                    x: {{
                        title: {{
                            display: true,
                            text: 'Document Size'
                        }}
                    }},
                    y: {{
                        beginAtZero: true,
                        title: {{
                            display: true,
                            text: '{y_label}'
                        }}
                    }}
                }}
            }}
        }}"""
            
            # Create chart for original ID (By SKU tab)
            html += f"""
        new Chart(document.getElementById('{chart['id']}'), {chart_config});
        
"""
            # Create chart for each test type tab
            for test_type in test_types:
                if section['test_type'] == test_type:
                    tab_chart_id = f"{chart['id']}_tab_{test_type.replace('_', '')}"
                    html += f"""
        new Chart(document.getElementById('{tab_chart_id}'), {chart_config});
        
"""
    
    html += """
        // Tab switching function
        function openTab(evt, tabName) {
            // Hide all tab content
            var tabContents = document.getElementsByClassName("tab-content");
            for (var i = 0; i < tabContents.length; i++) {
                tabContents[i].classList.remove("active");
            }
            
            // Remove active class from all tab buttons
            var tabButtons = document.getElementsByClassName("tab-button");
            for (var i = 0; i < tabButtons.length; i++) {
                tabButtons[i].classList.remove("active");
            }
            
            // Show the selected tab and mark button as active
            document.getElementById(tabName).classList.add("active");
            evt.currentTarget.classList.add("active");
        }
    </script>
</body>
</html>
"""
    
    with output_path.open('w', encoding='utf-8') as f:
        f.write(html)
    
    print(f"Document size sweep HTML report generated: {output_path}")

def generate_report(results_dir, output_file):
    """Generate the established user-count sweep report from Benchly JSON results."""
    print(f"Reading benchmark results from: {results_dir}...")
    rows, report_files = load_bench_results(results_dir)
    print(f"Found {len(report_files)} benchmark report files")
    data = read_and_process_csv(rows=rows)
    print(f"Processed {len(data)} averaged test results")
    
    print("Generating user sweep charts...")
    chart_sections = generate_user_count_sweep_charts(data)
    total_charts = sum(len(section['charts']) for section in chart_sections)
    print(f"Created {len(chart_sections)} SKU sections with {total_charts} total charts")
    
    print(f"Generating {output_file}...")
    generate_user_sweep_html(data, chart_sections, output_file)
    print("Done!")

if __name__ == '__main__':
    repository_root = Path(__file__).resolve().parent.parent
    timestamp = datetime.now().astimezone().strftime('%Y%m%d_%H%M%S')
    parser = argparse.ArgumentParser(description='Generate a user-count sweep HTML report.')
    parser.add_argument(
        '--results-dir',
        default=repository_root / 'bench-results',
        type=Path,
        help='Benchly result directory (default: %(default)s)',
    )
    parser.add_argument(
        '--output',
        type=Path,
        help='Output HTML path (default: bench-results/reports/sweep_report_<timestamp>.html)',
    )
    args = parser.parse_args()
    output = args.output or args.results_dir / 'reports' / f'sweep_report_{timestamp}.html'
    generate_report(args.results_dir, output)
