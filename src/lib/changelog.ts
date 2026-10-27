/**
 * 内置更新日志：目前咕咕启动器只有 1.0.0 一条（这是首个正式版本，不存在 0.x 历史）。
 * 结构固定为 { version, date, sections: [{ title, items[] }] }；关于页只显示最近 CHANGELOG_LIMIT 条。
 */

export interface ChangelogSection {
  title: string;
  items: string[];
}

export interface ChangelogEntry {
  version: string;
  date: string;
  sections: ChangelogSection[];
}

/** 关于页最多显示的条数：现在是 1 条，留给以后发版用。 */
export const CHANGELOG_LIMIT = 5;

export const CHANGELOG: ChangelogEntry[] = [
  {
    version: "1.0.0",
    date: "2026-10-27",
    sections: [
      {
        title: "新增",
        items: [
          "实例管理：新建（含内存滑块）、启动、打开目录、右键菜单与统计条",
          "实例内页：模组 / 资源包 / 光影 / 存档管理（启用停用、批量操作、导出实例包、日志查看、实例设置）",
          "皮肤：3D 预览（动作、旋转缩放、深浅背景）、上传与切换、模型经典/纤细、披风管理",
          "账号：微软设备码登录与离线账号，一键切换当前账号",
          "内容服务：Modrinth / CurseForge / MCIM 搜索、详情抽屉、版本与依赖、一键安装、内置翻译优先显示译文",
          "外观：背景四方案（渐变色 / 本地图 / API 地址 / Steam 壁纸引擎），互斥切换即时生效",
          "多人：陶瓦联机一键安装、开房与加入、房间码与在线玩家、运行日志",
          "娱乐：今日人品与每日一言，GitHub 下载器",
          "更新：手动与自动两种检查模式，检查结果显示在关于页",
          "下载源：官方 / BMCLAPI / 自定义镜像，测速与并发下载设置",
        ],
      },
      {
        title: "修复",
        items: ["连续弹两条提示时，前一条的定时器会把后一条提前清掉", "四档窗口宽度（1440 / 1280 / 1024 / 900）下的横向溢出"],
      },
    ],
  },
];
