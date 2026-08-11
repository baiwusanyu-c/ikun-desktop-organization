/**
 * 颜色工具函数 - 用于从基础主题色计算衍生颜色
 */

/**
 * 将十六进制颜色转换为 RGB
 */
export function hexToRgb(hex: string): { r: number; g: number; b: number } {
  const result = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})$/i.exec(hex);
  if (!result) return { r: 20, g: 184, b: 166 };
  return {
    r: parseInt(result[1], 16),
    g: parseInt(result[2], 16),
    b: parseInt(result[3], 16)
  };
}

/**
 * 调整颜色亮度
 */
export function adjustBrightness(hex: string, percent: number): string {
  const { r, g, b } = hexToRgb(hex);
  const clamp = (v: number) => Math.max(0, Math.min(255, Math.round(v)));
  const nr = clamp(r + (255 - r) * percent / 100);
  const ng = clamp(g + (255 - g) * percent / 100);
  const nb = clamp(b + (255 - b) * percent / 100);
  return "#" + [nr, ng, nb].map(x => x.toString(16).padStart(2, "0")).join("");
}

/**
 * 从基础主题色计算所有衍生颜色并设置 CSS 变量
 */
export function applyThemeColor(color: string): void {
  const { r, g, b } = hexToRgb(color);
  document.documentElement.style.setProperty("--primary-color", color);
  document.documentElement.style.setProperty("--primary-hover", adjustBrightness(color, -10));
  document.documentElement.style.setProperty("--primary-shadow", `rgba(${r}, ${g}, ${b}, 0.3)`);
  document.documentElement.style.setProperty("--primary-glow", `rgba(${r}, ${g}, ${b}, 0.15)`);
}