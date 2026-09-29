# 윈도우 코드서명 (SignPath Foundation, 오픈소스 무료)

- 서명 대상은 이 저장소의 GitHub Actions(`build.yml`)가 만든 산출물만. 맥 크로스컴파일본은 서명하지 않는다.
- 저장소 시크릿: `SIGNPATH_API_TOKEN`, `SIGNPATH_ORG_ID`. 프로젝트 슬러그 `nikh`, 정책 `release-signing`.
- 시크릿이 없으면 워크플로는 미서명 아티팩트만 남기고, 태그 릴리스엔 `-ci.exe` 접미로 올린다.
- 신청: https://signpath.org/ (Sign in with GitHub → Apply for OSS code signing) — 프로젝트 설명은 README 그대로.
