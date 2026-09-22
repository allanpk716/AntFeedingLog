# 票 04 · 头像裁剪编辑器(大图入口 + 拖动缩放 + 轻提示)

## What to build
巢况时间线大图查看器内的「调整头像裁剪」:
1. 大图查看器新增「调整头像裁剪」入口,打开裁剪编辑器(新组件):照片在方形裁剪框内可拖动平移、可缩放(桌面:滚轮/滑杆+鼠标拖动;手机网页:双指缩放+单指拖动)。
2. 编辑器产出归一化方形裁剪(x/y/边长);有"重置居中"操作;实时预览当前形状遮罩下的效果。
3. 保存调用票 01 的 update_photo_crop;保存成功按「轻提示」规范给"已保存"提示(评审建议 F5:即使调的不是当前头像那张、界面无天然变化,也必须有提示);失败红色带原因。
4. 保存后:该照片元数据更新,时间线/头像相关消费方经既有 saved→refresh 通路跟上;编辑器关闭回到大图。
5. 上传流程零变化:不自动弹编辑器(新照片默认居中)。

## 验收标准
- [ ] 大图查看器有「调整头像裁剪」;进入编辑器可拖动+缩放,裁剪框视觉为方形(形状遮罩仅预览)
- [ ] 保存后元数据读回与所调一致;重置居中后读回为空(NULL 语义)
- [ ] 保存成功有轻提示(含调整非头像照片场景);失败红色带原因可关
- [ ] 桌面鼠标与手机触摸(单指拖/双指缩)均可操作;触摸时不误触发页面滚动
- [ ] 上传照片后不自动弹编辑器
- [ ] PhotoCropEditor 新组件测试(产出裁剪/重置/保存调用/提示)+ NestCheckinDialog 集成用例(入口出现、打开编辑器)先红后绿;vue-tsc 0 错

## Blocked by
票 01(update_photo_crop 命令与裁剪字段)。

## 涉及路径
- src/components/PhotoCropEditor.vue(新建)
- src/components/PhotoCropEditor.test.ts(新建)
- src/components/NestCheckinDialog.vue
- src/components/NestCheckinDialog.test.ts

## 副作用声明
- 默认只跑类型检查与单文件组件测试(npx vitest run src/components/PhotoCropEditor.test.ts src/components/NestCheckinDialog.test.ts);不联网、不装依赖

decision_refs: D3, D4, D9(不做上传打断) · review_blocks: 无(F5 在本票落实)
