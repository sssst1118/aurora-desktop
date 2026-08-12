/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{vue,ts,tsx}"],
  // Phase4 4.4 主题:class 策略(html.dark 由 theme.ts 按配置添加;不用 media 自动跟随,深浅色由用户/配置决定)
  darkMode: "class",
  theme: {
    extend: {},
  },
  plugins: [],
}

