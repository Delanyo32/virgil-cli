// Exported symbols
export function greet(name: string): string {
  return `Hello, ${name}!`;
}

export class UserService {
  getName(): string {
    return "user";
  }
}

export const API_URL: string = "https://api.example.com";

export const fetchData = async (url: string) => {
  return fetch(url);
};

export interface User {
  id: number;
  name: string;
}

export type UserId = number;

export enum Role {
  Admin,
  User,
  Guest,
}

// Non-exported symbols
function helper(): void {}

const internalHandler = () => {
  console.log("internal");
};

// Destructured variable (should be skipped)
const { a, b } = { a: 1, b: 2 };
