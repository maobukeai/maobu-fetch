"""Public, anonymous metadata probes; never downloads media or reads browser cookies.

Run: python scripts/audit/media_probe.py --output media-results.json
Requires an already installed yt-dlp. This does not install or update tools.
An extractor success is not a completed-download or site-wide compatibility pass.
"""

import argparse
import concurrent.futures
import datetime
import json
import pathlib
import re
import shutil
import subprocess
import sys


SAMPLES = [
    ("douyin", "https://www.douyin.com/video/7694589931460594953"),
    ("bilibili", "https://www.bilibili.com/video/BV13x41117TL"),
    ("bilibili", "https://www.bilibili.com/video/BV1bK411W797"),
    ("youtube", "https://www.youtube.com/watch?v=YE7VzlLtp-4"),
    ("tiktok", "https://www.tiktok.com/@leenabhushan/video/6748451240264420610"),
    ("twitter", "https://twitter.com/starwars/status/665052190608723968"),
    ("weibo", "https://m.weibo.cn/status/4189191225395228"),
    ("xiaohongshu", "https://www.xiaohongshu.com/explore/6411cf99000000001300b6d9"),
    ("ixigua", "https://www.ixigua.com/6996881461559165471"),
    ("kuaishou", "https://www.kuaishou.com/short-video/3x5kdtj4t9fqws6"),
]
USER_AGENT = (
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
    "(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
)


def scrub_error(stderr):
    # Never preserve signed media URLs or authentication values in reports.
    text = "; ".join(stderr.strip().splitlines()[-2:])
    text = re.sub(r"https?://[^\s]+", "[URL]", text)
    text = re.sub(
        r"(?i)(cookie|authorization|token|password|secret|signature|sign)\s*[:=]\s*[^\s;]+",
        r"\1=[redacted]",
        text,
    )
    return text[:1200]


def probe(executable, version, sample):
    platform, url = sample
    result = {
        "platform": platform,
        "url": url,
        "time_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "tool_version": version,
        "cookie": "none",
        "mode": "anonymous metadata only; no config; no format URL check",
    }
    command = [
        executable, "--ignore-config", "--dump-single-json", "--no-playlist",
        "--no-warnings", "--no-check-formats", "--skip-download",
        "--socket-timeout", "8", "--retries", "0", "--extractor-retries", "0",
        "--user-agent", USER_AGENT,
    ]
    if platform == "youtube":
        command += [
            "--extractor-args", "youtube:player_client=default,mweb,web,android_vr,ios",
        ]
    if platform == "douyin":
        command += ["--referer", "https://live.douyin.com/"]
    command.append(url)
    try:
        process = subprocess.run(
            command, capture_output=True, text=True, encoding="utf-8",
            errors="replace", timeout=45, check=False,
        )
        result["exit_code"] = process.returncode
        if process.returncode == 0:
            value = json.loads(process.stdout)
            result.update(
                extractor=value.get("extractor_key"),
                format_count=len(value.get("formats") or []),
                title_present=bool(value.get("title")),
                drm=value.get("_has_drm"),
            )
        else:
            result["error_summary"] = scrub_error(process.stderr)
    except subprocess.TimeoutExpired:
        result["error_summary"] = "test harness timeout after 45 seconds"
    except (OSError, ValueError) as error:
        result["error_summary"] = scrub_error(str(error))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", default=shutil.which("yt-dlp"))
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    if not args.executable:
        parser.error("yt-dlp was not found; supply an already installed executable")
    version = subprocess.run(
        [args.executable, "--ignore-config", "--version"], capture_output=True,
        text=True, encoding="utf-8", timeout=10, check=True,
    ).stdout.strip()
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        results = list(pool.map(lambda sample: probe(args.executable, version, sample), SAMPLES))
    report = {
        "scope": "yt-dlp fallback only; excludes Maobu native Douyin/Twitter APIs and full downloads",
        "tool_version": version,
        "results": results,
    }
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    for row in results:
        print(row["platform"], row.get("exit_code", "timeout"), row.get("format_count", ""))
    return 0 if all(row.get("exit_code") == 0 for row in results) else 1


if __name__ == "__main__":
    sys.exit(main())
