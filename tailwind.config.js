/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        ink: "#0e1116",
        panel: "#171b22",
        edge: "#262c36",
        accent: "#38d39f",
        warn: "#e8b53a",
        bad: "#e35d5d",
      },
    },
  },
  plugins: [],
};
