"""diff_crawler: Compare two ML project directories.

Layers:
  1. Tree diff   - which paths exist in A, B, or both
  2. Code diff   - unified diff + similarity for text/code files
  3. Data diff   - row counts + schema diff via DuckDB
  4. Rename/move detection - MinHash LSH over file content
  5. Aggregate similarity report
"""

from .crawler import DiffCrawler, CrawlConfig
from .report import render_markdown

__all__ = ["DiffCrawler", "CrawlConfig", "render_markdown"]
__version__ = "0.1.0"
