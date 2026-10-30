use anyhow::Context as _;
use pesde_core::jiff::Timestamp;
use sqlx::MySqlConnection;
use sqlx::QueryBuilder;
use sqlx::query_builder::Separated;

macro_rules! insert_chunked {
	($conn:expr, $table:literal, $columns:expr, $rows:expr, $bind:expr $(,)?) => {
		$crate::util::insert_chunked_impl::<{ $columns.len() }, _>(
			$conn,
			const_str::concat!(
				"INSERT INTO ",
				$table,
				" (",
				const_str::join!(&$columns, ", "),
				") VALUES "
			),
			$rows,
			$bind,
		)
	};
}
pub(crate) use insert_chunked;

pub(crate) async fn insert_chunked_impl<const COLUMNS: usize, T>(
	conn: &mut MySqlConnection,
	prefix: &'static str,
	rows: impl IntoIterator<Item = T>,
	mut bind: impl FnMut(Separated<'_, sqlx::MySql, &'static str>, T),
) -> sqlx::Result<()> {
	#[expect(clippy::integer_division)]
	let max = const { (u16::MAX as usize / COLUMNS) - 1 };

	let mut rows = rows.into_iter();

	let mut query = QueryBuilder::new(prefix);
	while let Some(tpl) = rows.next() {
		query.push_tuples(std::iter::once(tpl).chain((&mut rows).take(max)), &mut bind);
		query.build().execute(&mut *conn).await?;
		query.reset();
	}

	Ok(())
}

pub(crate) fn chrono_to_jiff(
	chrono: sqlx::types::chrono::NaiveDateTime,
) -> anyhow::Result<Timestamp> {
	Timestamp::from_microsecond(chrono.and_utc().timestamp_micros()).map_err(Into::into)
}

pub(crate) fn jiff_to_chrono(
	jiff: pesde_core::jiff::Timestamp,
) -> anyhow::Result<sqlx::types::chrono::DateTime<sqlx::types::chrono::Utc>> {
	sqlx::types::chrono::DateTime::from_timestamp_micros(jiff.as_microsecond())
		.context("illegal timestamp")
}
