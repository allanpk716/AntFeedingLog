# 票 08 · 网页端照片上传与手机适配

规格:Implementation Decisions E(网页侧)、H;User Stories 9/10

## What to build

- HTTP multipart 照片上传端点:复用票 07 校验链;服务端生成文件名;受双闸与凭证保护。
- 照片读取端点:Bearer 凭证、`Content-Type: image/jpeg`+`X-Content-Type-Options: nosniff`+`Content-Disposition: inline`,仅以图片身份渲染。
- 手机竖屏适配:首页窝卡片、打卡面板、巢况登记(含拍照 input capture)、照片网格;不引入第二套界面(同一套组件响应式适配)。
- 网页端巢况照片上传与展示闭环。

## 验收标准

- [ ] 浏览器上传→时间线展示闭环;超限/坏格式被 400
- [ ] 照片响应头三件套正确;无凭证取图 401
- [ ] 手机视口(375px 宽)布局可用(组件测试/截图)
- [ ] 测试齐

## Blocked by

票 05、票 07
