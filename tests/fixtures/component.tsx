interface Props {
  title: string;
}

export function App(props: Props) {
  return <div>{props.title}</div>;
}

export const Header = () => {
  return <h1>Header</h1>;
};
